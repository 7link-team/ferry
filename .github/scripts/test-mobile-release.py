"""Native-package fixtures exercise the release gate without Android/Xcode."""
import importlib.util
import json
import pathlib
import plistlib
import tempfile
import unittest
import zipfile
from unittest.mock import patch

SCRIPT = pathlib.Path(__file__).with_name('mobile-release.py')
spec = importlib.util.spec_from_file_location('mobile_release', SCRIPT)
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class MobileReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        (self.root / 'desktop').mkdir()
        (self.root / 'package.json').write_text(json.dumps({'name': 'client', 'version': '1.2.3-alpha.1'}))
        (self.root / 'desktop/tauri.conf.json').write_text(json.dumps({
            'identifier': 'org.example.client', 'version': '1.2.3-alpha.1'}))
        self.output = self.root / 'output'
        self.output.mkdir()

    def archive(self, path, entries):
        path.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(path, 'w') as package:
            for name, data in entries.items():
                package.writestr(name, data)
        return path

    def apk(self, abi):
        return self.archive(self.root / f'{abi}.apk', {
            'AndroidManifest.xml': b'manifest', 'classes.dex': b'dex',
            f'lib/{abi}/libapp.so': b'native-client'})

    def apple(self, platform, ipa=False):
        prefix = 'Payload/Client.app/' if ipa else 'Client.app/'
        info = plistlib.dumps({'CFBundleIdentifier': 'org.example.client',
            'CFBundleShortVersionString': '1.2.3', 'CFBundleExecutable': 'Client',
            'CFBundleSupportedPlatforms': [platform]})
        return self.archive(self.root / ('client.ipa' if ipa else 'simulator.zip'), {
            prefix + 'Info.plist': info, prefix + 'Client': b'native-client'})

    def stage_all(self):
        config = release.configuration(self.root, 'refs/tags/v1.2.3-alpha.1')
        with patch.object(release.subprocess, 'check_output', return_value=(
                "package: name='org.example.client' versionName='1.2.3-alpha.1'\n")):
            release.stage_android(config, [self.apk('arm64-v8a'), self.apk('x86_64')], self.output, 'aapt')
        release.stage_ios(config, self.apple('iPhoneOS', True), self.apple('iPhoneSimulator'), self.output)
        return config

    def test_complete_release_has_versioned_assets_and_verified_hashes(self):
        config = self.stage_all()
        release.verify(config, self.output)
        self.assertEqual(len(list(self.output.glob('*.apk'))), 2)
        self.assertEqual(len(list(self.output.glob('*.ipa'))), 1)
        self.assertTrue(all('1.2.3-alpha.1' in p.name for p in self.output.iterdir()))
        disclosure = next(self.output.glob('*SIGNING*.txt')).read_text()
        self.assertIn('unsigned', disclosure)
        self.assertIn('persistent release key', disclosure)
        self.assertFalse(any(p.name.endswith('-debug.apk') for p in self.output.iterdir()))

    def test_missing_architecture_blocks_publication(self):
        config = self.stage_all()
        next(self.output.glob('*x86_64*.apk')).unlink()
        with self.assertRaisesRegex(ValueError, 'missing|Missing'):
            release.verify(config, self.output)

    def test_modified_asset_blocks_publication(self):
        config = self.stage_all()
        next(self.output.glob('*.ipa')).write_bytes(b'corrupted')
        with self.assertRaisesRegex(ValueError, 'hash|checksum'):
            release.verify(config, self.output)

    def test_tag_must_match_both_project_versions(self):
        with self.assertRaisesRegex(ValueError, 'version|Version'):
            release.configuration(self.root, 'refs/tags/v1.2.2')
        (self.root / 'desktop/tauri.conf.json').write_text(json.dumps({
            'identifier': 'org.example.client', 'version': '1.2.2'}))
        with self.assertRaisesRegex(ValueError, 'version|Version'):
            release.configuration(self.root, '')

    def test_wrong_apk_identity_or_version_is_rejected(self):
        config = release.configuration(self.root, '')
        for metadata in ["name='other' versionName='1.2.3-alpha.1'",
                         "name='org.example.client' versionName='1.2.2'"]:
            with self.subTest(metadata=metadata), patch.object(release.subprocess, 'check_output', return_value=metadata):
                with self.assertRaisesRegex(ValueError, 'identity|version'):
                    release.stage_android(config, [self.apk('arm64-v8a'), self.apk('x86_64')], self.output, 'aapt')

    def test_debug_builds_and_foreign_signers_are_rejected(self):
        config = release.configuration(self.root, '')
        good = "package: name='org.example.client' versionName='1.2.3-alpha.1'\n"
        with patch.object(release.subprocess, 'check_output', return_value=good + "application-debuggable\n"):
            with self.assertRaisesRegex(ValueError, 'release build'):
                release.stage_android(config, [self.apk('arm64-v8a'), self.apk('x86_64')], self.output, 'aapt')
        signed = "Signer #1 certificate SHA-256 digest: " + "a" * 64 + "\n"
        with patch.object(release.subprocess, 'check_output', side_effect=lambda command, **_: signed if 'verify' in command else good):
            with self.assertRaisesRegex(ValueError, 'persistent release key'):
                release.stage_android(config, [self.apk('arm64-v8a'), self.apk('x86_64')], self.output, 'aapt', 'apksigner', 'b' * 64)
            release.stage_android(config, [self.apk('arm64-v8a'), self.apk('x86_64')], self.output, 'aapt', 'apksigner', 'a' * 64)

    def test_sidecar_and_missing_native_client_are_rejected(self):
        for entries in [{'AndroidManifest.xml': b'x', 'classes.dex': b'x'},
                        {'ferry-server': b'server', 'lib/arm64-v8a/libapp.so': b'x'}]:
            path = self.archive(self.root / 'bad.apk', entries)
            with self.assertRaises(ValueError):
                release.android_abi(path)

    def test_wrong_apple_version_platform_or_missing_binary_is_rejected(self):
        config = release.configuration(self.root, '')
        for info in [
            {'CFBundleShortVersionString': '1.2.2'},
            {'CFBundleSupportedPlatforms': ['iPhoneSimulator']},
            {'CFBundleExecutable': 'Missing'},
            {'CFBundleIdentifier': 'other'},
        ]:
            path = self.apple('iPhoneOS', True)
            with zipfile.ZipFile(path) as package:
                entries = {n: package.read(n) for n in package.namelist()}
            plist = plistlib.loads(entries['Payload/Client.app/Info.plist'])
            plist.update(info)
            entries['Payload/Client.app/Info.plist'] = plistlib.dumps(plist)
            self.archive(path, entries)
            with self.subTest(info=info), self.assertRaises(ValueError):
                release.apple_contents(config, path, 'iPhoneOS')


if __name__ == '__main__':
    unittest.main()

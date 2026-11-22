"""Validate and stage native mobile packages for the shared release collector."""
import argparse
import hashlib
import json
import os
import pathlib
import plistlib
import re
import shutil
import subprocess
import zipfile


def require(condition, message):
    if not condition:
        raise ValueError(message)


def configuration(root, ref):
    project = json.loads((root / 'package.json').read_text(encoding='utf-8'))
    native = json.loads((root / 'desktop/tauri.conf.json').read_text(encoding='utf-8'))
    version = project['version']
    require(re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?', version), 'Invalid version')
    require(version == native['version'], 'Project and native versions differ')
    if ref.startswith('refs/tags/'):
        require(ref == f'refs/tags/v{version}', 'Tag and package version differ')
    require(re.fullmatch(r'[A-Za-z0-9._-]+', project['name']), 'Invalid package name')
    return {**native, 'name': project['name'], 'version': version}


def names(config):
    stem = f"{config['name']}-v{config['version']}"
    return {
        'arm64-v8a': f'{stem}-android-arm64-v8a-debug.apk',
        'x86_64': f'{stem}-android-x86_64-debug.apk',
        'device': f'{stem}-ios-arm64-unsigned.ipa',
        'simulator': f'{stem}-ios-arm64-simulator.app.zip',
        'signing': f'{stem}-mobile-SIGNING.txt',
    }


def contents(package):
    require(package.testzip() is None, 'Package ZIP integrity check failed')
    require(not any(any(value in name.lower() for value in
        ('ferry-server', 'ferry-sandbox', 'actions-worker-')) for name in package.namelist()),
        'Mobile package contains a server sidecar')


def android_abi(path):
    with zipfile.ZipFile(path) as package:
        contents(package)
        for name in ('AndroidManifest.xml', 'classes.dex'):
            require(name in package.namelist() and package.getinfo(name).file_size > 0,
                    f'Missing Android client content: {name}')
        abis = {name.split('/')[1] for name in package.namelist()
                if re.fullmatch(r'lib/[^/]+/[^/]+\.so', name) and package.getinfo(name).file_size > 0}
        require(len(abis) == 1 and abis <= {'arm64-v8a', 'x86_64'}, 'Missing or unexpected native Android ABI')
        return abis.pop()


def apple_contents(config, path, platform):
    with zipfile.ZipFile(path) as package:
        contents(package)
        pattern = r'Payload/[^/]+\.app/Info.plist' if platform == 'iPhoneOS' else r'[^/]+\.app/Info.plist'
        plists = [name for name in package.namelist() if re.fullmatch(pattern, name)]
        require(len(plists) == 1, 'Expected one native Apple app')
        info = plistlib.loads(package.read(plists[0]))
        require(info.get('CFBundleIdentifier') == config['identifier'], 'Apple package identity mismatch')
        # Apple's short bundle version is numeric; the release filename retains
        # the full semver prerelease suffix from the checked-out tag/config.
        numeric = config['version'].split('-')[0].split('+')[0]
        require(info.get('CFBundleShortVersionString') == numeric, 'Apple package version mismatch')
        require(info.get('CFBundleSupportedPlatforms') == [platform], 'Apple package platform mismatch')
        binary = plists[0].rsplit('/', 1)[0] + '/' + info.get('CFBundleExecutable', '')
        require(binary in package.namelist() and package.getinfo(binary).file_size > 0,
                'Missing native Apple client executable')


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def checksum(path):
    path.with_name(path.name + '.sha256').write_text(f'{digest(path)}  {path.name}\n', encoding='utf-8')


def stage_android(config, apks, output, aapt):
    require(len(apks) == 2, 'Exactly two split debug APKs are required')
    found = set()
    for apk in apks:
        abi = android_abi(apk)
        require(abi not in found, 'Duplicate Android ABI')
        found.add(abi)
        metadata = subprocess.check_output([str(aapt), 'dump', 'badging', str(apk)], text=True)
        require(f"name='{config['identifier']}'" in metadata, 'Android package identity mismatch')
        require(f"versionName='{config['version']}'" in metadata, 'Android package version mismatch')
        target = output / names(config)[abi]
        shutil.copyfile(apk, target)
        checksum(target)


def stage_ios(config, ipa, simulator, output):
    for key, source, platform in [('device', ipa, 'iPhoneOS'), ('simulator', simulator, 'iPhoneSimulator')]:
        apple_contents(config, source, platform)
        target = output / names(config)[key]
        shutil.copyfile(source, target)
        checksum(target)
    disclosure = output / names(config)['signing']
    disclosure.write_text(
        'Android APKs are debug builds signed with the CI debug key, not production release signing.\n'
        'The simulator archive runs only in an ARM64 iOS simulator.\n'
        'The device IPA is unsigned and cannot be installed on a physical iPhone until signed '
        'with a valid Apple team and provisioning profile. No App Store publication was performed.\n',
        encoding='utf-8')
    checksum(disclosure)


def verify(config, output):
    for key, name in names(config).items():
        path = output / name
        require(path.is_file() and path.stat().st_size > 0, f'Missing mobile release asset: {name}')
        sums = path.with_name(path.name + '.sha256')
        require(sums.is_file(), f'Missing mobile checksum: {name}')
        require(sums.read_text(encoding='utf-8') == f'{digest(path)}  {name}\n', f'Mobile checksum mismatch: {name}')
        if key in ('arm64-v8a', 'x86_64'):
            require(android_abi(path) == key, 'Android ABI mismatch')
        elif key in ('device', 'simulator'):
            apple_contents(config, path, 'iPhoneOS' if key == 'device' else 'iPhoneSimulator')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['android', 'ios', 'verify', 'version'])
    parser.add_argument('--output', type=pathlib.Path, default=pathlib.Path('release-assets'))
    args = parser.parse_args()
    root = pathlib.Path.cwd()
    config = configuration(root, os.environ.get('GITHUB_REF', ''))
    if args.mode == 'version':
        print(config['version'])
        return
    args.output.mkdir(parents=True, exist_ok=True)
    if args.mode == 'android':
        apks = list((root / 'desktop/gen/android/app/build/outputs/apk').rglob('*.apk'))
        aapt = pathlib.Path(os.environ['ANDROID_HOME']) / 'build-tools/36.0.0/aapt'
        stage_android(config, apks, args.output, aapt)
    elif args.mode == 'ios':
        ipas = list((root / 'desktop/gen/apple/build/arm64').glob('*.ipa'))
        require(len(ipas) == 1, 'One unsigned device IPA is required')
        stage_ios(config, ipas[0], root / 'build-artifacts/mobile-ios/simulator-client.app.zip', args.output)
    else:
        verify(config, args.output)


if __name__ == '__main__':
    main()

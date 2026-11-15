"""Failure contracts; no video service or tool download required."""
import copy
import hashlib
import base64
import unittest
from unittest.mock import patch

import video


class VideoContracts(unittest.TestCase):
    def test_corrupt_tool_archive_is_rejected(self):
        integrity = "sha512-" + base64.b64encode(hashlib.sha512(b"original").digest()).decode()
        video.verified_package(b"original", integrity)
        with self.assertRaisesRegex(RuntimeError, "integrity mismatch"):
            video.verified_package(b"corrupt", integrity)

    def test_platform_selection_and_unsupported_platform(self):
        for system, architecture, expected in [
            ("Windows", "AMD64", "windows-x86_64"),
            ("Linux", "x86_64", "linux-x86_64"),
        ]:
            with patch("platform.system", return_value=system), patch("platform.machine", return_value=architecture):
                self.assertEqual(video.platform_key(), expected)
        with patch("platform.machine", return_value="aarch64"):
            with self.assertRaisesRegex(RuntimeError, "Unsupported architecture"):
                video.platform_key()
        with patch("platform.machine", return_value="x86_64"), patch("platform.system", return_value="Darwin"):
            with self.assertRaisesRegex(RuntimeError, "Unsupported system"):
                video.platform_key()

    def test_media_requirement_rejects_invalid_results(self):
        valid = {
            "streams": [{"codec_type": "video", "width": 1280, "height": 720,
                         "codec_name": "h264", "avg_frame_rate": "30/1", "nb_frames": "150"}],
            "format": {"duration": "5.000000"},
        }
        video.check_media(valid)
        for field, value in [("width", 640), ("codec_name", "hevc"), ("nb_frames", "149"), ("avg_frame_rate", "24/1")]:
            invalid = copy.deepcopy(valid)
            invalid["streams"][0][field] = value
            with self.assertRaises(RuntimeError):
                video.check_media(invalid)
        invalid = copy.deepcopy(valid)
        invalid["format"]["duration"] = "4.9"
        with self.assertRaisesRegex(RuntimeError, "five seconds"):
            video.check_media(invalid)
        invalid = copy.deepcopy(valid)
        invalid["streams"].append({"codec_type": "audio"})
        with self.assertRaisesRegex(RuntimeError, "no audio"):
            video.check_media(invalid)


if __name__ == "__main__":
    unittest.main()

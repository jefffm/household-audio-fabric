import array
import os
import subprocess
import sys
import tempfile
import unittest
import wave
from pathlib import Path


class SpeechContractTest(unittest.TestCase):
    @unittest.skipUnless(os.environ.get("ANNOUNCER_TEST_MODEL"), "set ANNOUNCER_TEST_MODEL for real Piper synthesis")
    def test_real_speech_is_bounded_pcm_and_overlong_clip_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "speech.wav"
            command = [sys.executable, "-m", "announcer.synthesize", "--model", os.environ["ANNOUNCER_TEST_MODEL"], "--output", str(target)]
            result = subprocess.run(command + ["--max-seconds", "10"], input=b"The side door opened.", capture_output=True, timeout=25)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            with wave.open(str(target)) as wav:
                self.assertEqual((wav.getnchannels(), wav.getsampwidth(), wav.getframerate()), (1, 2, 22050))
                self.assertGreater(wav.getnframes(), 22050)
                self.assertLessEqual(wav.getnframes(), 220500)
                samples = array.array("h", wav.readframes(wav.getnframes()))
                peak = max(abs(sample) for sample in samples)
                self.assertGreater(peak, 500)
                self.assertLessEqual(peak, 3932)
            rejected = Path(directory) / "rejected.wav"
            command[command.index(str(target))] = str(rejected)
            result = subprocess.run(command + ["--max-seconds", "0.1"], input=b"The side door opened.", capture_output=True, timeout=25)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(rejected.exists())
            self.assertNotIn(b"side door", result.stderr)


if __name__ == "__main__":
    unittest.main()

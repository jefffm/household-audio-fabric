"""One bounded CPU synthesis child. Text arrives only on stdin, never argv/logs."""

import argparse
import json
import sys
import wave
from pathlib import Path

import numpy as np
import onnxruntime
from piper import PiperVoice
from piper.config import PiperConfig


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--max-seconds", type=float, default=10)
    args = parser.parse_args()
    try:
        text = sys.stdin.read(2049)
        if not 1 <= len(text) <= 240 or not 0 < args.max_seconds <= 10:
            raise ValueError("invalid input")
        onnxruntime.disable_telemetry_events()
        options = onnxruntime.SessionOptions()
        options.intra_op_num_threads = 2
        options.inter_op_num_threads = 1
        voice = PiperVoice(
            config=PiperConfig.from_dict(json.loads(Path(args.model + ".json").read_text())),
            session=onnxruntime.InferenceSession(args.model, sess_options=options, providers=["CPUExecutionProvider"]),
        )
        chunks = []
        count = 0
        for chunk in voice.synthesize(text):
            count += len(chunk.audio_int16_array)
            if count > voice.config.sample_rate * args.max_seconds:
                raise ValueError("clip too long")
            chunks.append(chunk.audio_int16_array)
        if not chunks or not count:
            raise ValueError("empty clip")
        samples = np.concatenate(chunks).astype(np.float64)
        peak = float(np.max(np.abs(samples)))
        samples *= min(1.0, 3932 / max(1, peak))
        with wave.open(args.output, "wb") as target:
            target.setparams((1, 2, voice.config.sample_rate, 0, "NONE", "not compressed"))
            target.writeframes(samples.astype("<i2").tobytes())
    except Exception:
        Path(args.output).unlink(missing_ok=True)
        print("synthesis_failed", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

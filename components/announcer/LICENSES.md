# Announcement engine and voice licenses

Original announcer code is MIT (LICENSE). This does not relicense the image.

- Piper TTS 1.8.0 is GPL-3.0-or-later. Corresponding engine source: https://github.com/OHF-Voice/piper1-gpl/tree/v1.8.0 . Its wheel includes separately licensed eSpeak NG material and license files. Preserve the installed distribution's notices under `/usr/local/lib/python3.12/site-packages/` and comply with GPL source obligations when redistributing this combined image.
- Paho MQTT 2.1.0 is dual EPL-2.0 / EDL-1.0. Source: https://github.com/eclipse-paho/paho.mqtt.python/tree/v2.1.0 . Its distribution license is retained alongside the installed package.
- ONNX Runtime, NumPy and other transitive libraries retain their own installed package licenses. Exact versions/hashes are in requirements.lock; Python/Debian packages retain upstream notices. Inspect the full image inventory before redistribution, not just this list.
- LJSpeech medium voice is by Bryce Beattie, whose model release page explicitly dedicates medium/high LJSpeech models to the public domain: https://brycebeattie.com/files/tts/ . Corpus licensing: https://keithito.com/LJ-Speech-Dataset/ . Immutable artifact revision and SHA-256 values are in voices.lock.json. Model card: https://huggingface.co/rhasspy/piper-voices/blob/c10ece1aade47bb51c153c893d14e5bf8e5b7117/en/en_US/ljspeech/medium/MODEL_CARD .

No license conclusion is inferred from the voice repository's blanket metadata. Do not substitute another voice without checking its model and dataset terms. This image has GPL-covered components; it is not an MIT-only distribution.

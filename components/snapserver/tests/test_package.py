import hashlib
import pathlib
import unittest
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
RELEASE = "https://github.com/snapcast/snapcast/releases/download/v0.35.0"
HASHES = {
    "snapserver_0.35.0-1_amd64_bookworm.deb": "afb012318bb8cbf82c63c2084de3b290f53108c49976ff364100882cd8816b41",
    "snapserver_0.35.0-1_arm64_bookworm.deb": "526cded407fad5940e834d77cf66a3b7ebac68931a530be55179f60d327044fd",
    "snapserver_0.35.0-1_armhf_bookworm.deb": "3c05ae3057182c1c3bb8cd1f4ae4ad0500f30bae04147765ea4704f3d00b646a",
    "snapclient_0.35.0-1_amd64_bookworm.deb": "b71575bcc2b4541f5004c73a0a1f43374e852bf5d5f2162aa1b230f3c209d9f3",
    "snapclient_0.35.0-1_arm64_bookworm.deb": "83afa0910cce99c0e6d4a52ec1849240c9956f5147e0743d60d4dd5f3b11af1a",
    "snapclient_0.35.0-1_armhf_bookworm.deb": "b532928974d5fa1bef8aa44e7400fb47ee3e91cede319c90cf830d23cf18ddb2",
}

class PackageContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.conf = (ROOT / "snapserver.conf").read_text()
        cls.container = (ROOT / "Containerfile").read_text()
        cls.compose = (ROOT / "compose.yaml").read_text()

    def test_three_fixed_flac_sources(self):
        for value in ("name=idle&mode=create", "name=AirPlay&mode=server", "name=MA-Test-PCM&mode=server"):
            self.assertIn(value + "&sampleformat=48000:32:2&codec=flac", self.conf)
        self.assertIn("default_source = idle", self.conf)

    def test_ports_are_alternate_and_loopback_published(self):
        for port in ("11704", "11705", "11780", "14953", "14954"):
            self.assertIn(f'127.0.0.1:${{', self.compose)
            self.assertIn(f':{port}"', self.compose)
        for production_default in ("port = 1704", "port = 1705", "port = 1780"):
            self.assertNotIn(production_default, self.conf)

    def test_hardened_minimal_server_target(self):
        server = self.container.split("FROM runtime-base AS server", 1)[1]
        test = self.container.split("FROM runtime-base AS test", 1)[1].split("FROM runtime-base AS server", 1)[0]
        self.assertIn("USER snapserver:snapserver", server)
        self.assertNotIn("snapclient", server)
        self.assertNotIn("curl", server)
        self.assertIn("snapclient", test)
        for setting in ('cap_drop: ["ALL"]', 'no-new-privileges:true', "read_only: true"):
            self.assertIn(setting, self.compose)

    def test_snapshot_and_exact_dependency_lock(self):
        self.assertIn("SNAPSHOT=20260810T000000Z", self.container)
        lock = (ROOT / "packages.lock").read_text().splitlines()
        self.assertTrue(lock)
        self.assertTrue(all("=" in line and not line.endswith("=") for line in lock))
        self.assertIn("bookworm-slim@sha256:abd67", self.container)
        self.assertIn('org.opencontainers.image.source="https://github.com/jefffm/household-audio-fabric"', self.container)

    def test_release_artifact_hashes_are_real(self):
        for filename, expected in HASHES.items():
            with self.subTest(filename=filename):
                with urllib.request.urlopen(f"{RELEASE}/{filename}", timeout=30) as response:
                    actual = hashlib.sha256(response.read()).hexdigest()
                self.assertEqual(actual, expected)
                self.assertIn(expected, self.container)

if __name__ == "__main__":
    unittest.main()

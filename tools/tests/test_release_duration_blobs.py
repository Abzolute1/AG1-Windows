import hashlib
import sys
import tempfile
import unittest
import zlib
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'release_duration'))
from workload_checks import logical_blob_digest

class LogicalBlobTests(unittest.TestCase):
    def test_raw_compressed_and_invalid_streams(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            body = b'custody payload\n' * 100000
            expected = hashlib.sha256(body).hexdigest()
            raw = root / expected
            raw.write_bytes(body)
            self.assertEqual(logical_blob_digest(root, expected), expected)
            raw.unlink()
            physical = b'AG1ZLIB1' + len(body).to_bytes(8, 'little') + zlib.compress(body, 1)
            compressed = root / (expected + '.z1')
            compressed.write_bytes(physical)
            self.assertEqual(logical_blob_digest(root, expected), expected)
            for bad in [physical[:8], physical[:-1], physical + b'0', physical + physical[16:],
                        b'UNKNOWN1' + physical[8:], physical[:8] + (2**40).to_bytes(8, 'little') + physical[16:],
                        physical[:8] + (len(body)-1).to_bytes(8, 'little') + physical[16:]]:
                compressed.write_bytes(bad)
                with self.assertRaises((RuntimeError, zlib.error)):
                    logical_blob_digest(root, expected)
            compressed.write_bytes(physical)
            raw.write_bytes(b'corrupt preferred raw')
            self.assertNotEqual(logical_blob_digest(root, expected), expected)

if __name__ == '__main__':
    unittest.main()

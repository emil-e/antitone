import hashlib
import unittest

from independent_validation import CANDIDATE, verify_freeze


class IndependentValidationFreeze(unittest.TestCase):
    def test_matching_manifest_and_candidate(self):
        cases = b'[{"name":"case"}]\n'
        freeze = {"case_manifest_sha256": hashlib.sha256(cases).hexdigest(), "candidate": CANDIDATE}
        self.assertEqual(verify_freeze(cases, freeze), [{"name": "case"}])

    def test_changed_manifest_or_candidate_rejects(self):
        cases = b"[]\n"
        digest = hashlib.sha256(cases).hexdigest()
        with self.assertRaises(ValueError):
            verify_freeze(b"[1]\n", {"case_manifest_sha256": digest, "candidate": CANDIDATE})
        with self.assertRaises(ValueError):
            verify_freeze(cases, {"case_manifest_sha256": digest, "candidate": "retuned"})


if __name__ == "__main__":
    unittest.main()

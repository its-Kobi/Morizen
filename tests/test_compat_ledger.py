import copy
import unittest

from tools import compat_ledger


class CompatLedgerTests(unittest.TestCase):
    def test_tracked_ledger_is_valid_and_evidence_weighted(self):
        result = compat_ledger.summary(compat_ledger.load())
        self.assertGreater(result["supported_weight"], 0)
        self.assertLess(result["supported_weight"], result["total_weight"])
        loader = next(area for area in result["areas"] if area["id"] == "loader")
        self.assertEqual(loader["status"], "partial")
        self.assertEqual(loader["evidence"], ["empty", "mixin-head"])

    def test_supported_member_requires_evidence(self):
        data = copy.deepcopy(compat_ledger.load())
        member = data["areas"][0]["members"][0]
        member.pop("evidence")
        with self.assertRaisesRegex(ValueError, "without test evidence"):
            compat_ledger.validate(data)

    def test_area_status(self):
        area = {"members": [{"status": "unsupported"}]}
        self.assertEqual(compat_ledger.area_status(area), "unsupported")
        area["members"].append({"status": "supported"})
        self.assertEqual(compat_ledger.area_status(area), "partial")


if __name__ == "__main__":
    unittest.main()

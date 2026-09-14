from pathlib import Path
import unittest


class RecoveryDrillTest(unittest.TestCase):
    def test_manual_drill_names_every_ruled_failure_injection(self):
        releasing = Path("RELEASING.md").read_text()
        drill = releasing.split("7. **Recovery drill", 1)[1].split(
            "8. **Stage everything", 1
        )[0]
        injections = {
            "forced process kill": "force-kill",
            "permission loss": "permission",
            "corrupt document": "corrupt",
            "partial aggregate completion": "half done",
            "storage exhaustion": "disk full",
            "operating-system restart": "OS restart",
        }

        missing = [label for label, marker in injections.items() if marker not in drill]
        self.assertEqual([], missing, f"recovery drill omits: {missing}")


if __name__ == "__main__":
    unittest.main()

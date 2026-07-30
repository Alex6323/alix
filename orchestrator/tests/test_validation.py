from __future__ import annotations

import unittest

from orchestrator.validation import validate_patch


class PatchValidationTests(unittest.TestCase):
    def test_forbidden_gate_and_orchestrator_changes_are_rejected(self) -> None:
        patch = """\
diff --git a/Makefile b/Makefile
--- a/Makefile
+++ b/Makefile
@@ -1 +1 @@
-gate: check
+gate:
diff --git a/orchestrator/src/orchestrator/engine.py b/orchestrator/src/orchestrator/engine.py
new file mode 100644
--- /dev/null
+++ b/orchestrator/src/orchestrator/engine.py
@@ -0,0 +1 @@
+pass
"""

        result = validate_patch(patch)

        self.assertTrue(result.rejected)
        self.assertIn("Makefile", " ".join(result.reasons))
        self.assertIn("orchestrator/", " ".join(result.reasons))

    def test_mutants_skip_is_rejected(self) -> None:
        patch = """\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1,2 @@
 fn add() {}
+#[mutants::skip]
"""

        result = validate_patch(patch)

        self.assertTrue(result.rejected)
        self.assertIn("mutants::skip", " ".join(result.reasons))

    def test_removed_test_lines_are_flagged_for_human_review_not_rejected(self) -> None:
        patch = """\
diff --git a/tests/add.rs b/tests/add.rs
--- a/tests/add.rs
+++ b/tests/add.rs
@@ -1,4 +1,3 @@
 #[test]
 fn adds() {
-    assert_eq!(4, add(2, 2));
 }
"""

        result = validate_patch(patch)

        self.assertFalse(result.rejected)
        self.assertTrue(result.needs_human_review)

    def test_replacing_a_test_assertion_is_not_a_deletion_flag(self) -> None:
        patch = """\
diff --git a/tests/add.rs b/tests/add.rs
--- a/tests/add.rs
+++ b/tests/add.rs
@@ -1,4 +1,4 @@
 #[test]
 fn adds() {
-    assert_eq!(4, add(2, 2));
+    assert_eq!(5, add(2, 3));
 }
"""

        result = validate_patch(patch)

        self.assertFalse(result.rejected)
        self.assertFalse(result.needs_human_review)

from __future__ import annotations

import unittest

from orchestrator.spec_api import ApiContractError, extract_rust_api


class ApiContractTests(unittest.TestCase):
    def test_extracts_the_rust_stub_only_from_the_api_section(self) -> None:
        spec = """\
# Feature
```rust
pub fn wrong() {}
```
## API
The property author compiles against this exact contract.
```rust
pub trait Counter {
    fn increment(&mut self) -> u64;
}
```
## Acceptance
Done.
"""

        self.assertEqual(
            "pub trait Counter {\n    fn increment(&mut self) -> u64;\n}\n",
            extract_rust_api(spec),
        )

    def test_missing_api_section_is_reported_as_a_spec_bug(self) -> None:
        with self.assertRaisesRegex(ApiContractError, "## API"):
            extract_rust_api("# Feature\nNo public contract.\n")

    def test_missing_rust_fence_is_reported_as_a_spec_bug(self) -> None:
        with self.assertRaisesRegex(ApiContractError, "rust"):
            extract_rust_api("## API\nProse only.\n")

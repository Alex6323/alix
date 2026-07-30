from __future__ import annotations

import re


class ApiContractError(ValueError):
    pass


def extract_rust_api(spec: str) -> str:
    section = re.search(
        r"(?ms)^## API[ \t]*\n(?P<body>.*?)(?=^## |\Z)",
        spec,
    )
    if section is None:
        raise ApiContractError(
            "asymmetric mode requires a `## API` section with an exact Rust stub"
        )
    fence = re.search(
        r"(?ms)^```rust[ \t]*\n(?P<code>.*?)^```[ \t]*$",
        section.group("body"),
    )
    if fence is None:
        raise ApiContractError(
            "the `## API` section must contain a fenced `rust` stub"
        )
    return fence.group("code").rstrip() + "\n"

#!/usr/bin/env python3
"""Validate agent-facing docs and capability metadata.

Ensures:
1. llmrust.capabilities.json is valid JSON with correct structure.
2. examples/README.md only lists examples that exist in Cargo.toml and examples/*.rs.
3. AGENTS.md and docs/CONTRACTS.md do not contain known incorrect statements.
4. Capability disclaimer exists in docs/CAPABILITIES.md.
5. proxy_server command includes --features proxy.

Exit code 0 = all checks passed. Non-zero = at least one check failed.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ERRORS = 0


def error(msg: str) -> None:
    global ERRORS
    print(f"  ERROR: {msg}", file=sys.stderr)
    ERRORS += 1


def check(name: str) -> None:
    print(f"  {name} ... ok")


# ── 1. Validate llmrust.capabilities.json ──────────────────────────

print("=== 1. llmrust.capabilities.json ===")

cap_path = ROOT / "llmrust.capabilities.json"
if not cap_path.exists():
    error("llmrust.capabilities.json not found")
else:
    try:
        cap = json.loads(cap_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        error(f"llmrust.capabilities.json is not valid JSON: {e}")
        cap = None

    if cap is not None:
        # Top-level fields
        if cap.get("name") != "llmrust":
            error(f'name must be "llmrust", got {cap.get("name")!r}')
        if not cap.get("version"):
            error("version is missing")
        else:
            check("name + version")
        providers = cap.get("providers")
        if not isinstance(providers, dict):
            error("providers must be a dict")
        else:
            required = {"openai", "deepseek", "moonshot", "openrouter", "anthropic", "google", "ollama"}
            missing = required - set(providers.keys())
            extra = set(providers.keys()) - required
            if missing:
                error(f"missing provider entries: {sorted(missing)}")
            if extra:
                error(f"unexpected provider entries: {sorted(extra)}")
            if not missing and not extra:
                check("provider entries complete")

        # Proxy auth config
        proxy = cap.get("proxy")
        if not isinstance(proxy, dict):
            error("proxy section missing or not a dict")
        else:
            auth = proxy.get("auth")
            if not isinstance(auth, dict):
                error("proxy.auth missing or not a dict")
            else:
                if auth.get("config") != "LLMRUST_PROXY_KEY":
                    error(f'proxy.auth.config must be "LLMRUST_PROXY_KEY", got {auth.get("config")!r}')
                else:
                    check("proxy auth config")

            # n_policy must be correct
            n_policy = proxy.get("n_policy", "")
            if not isinstance(n_policy, str):
                error("proxy.n_policy must be a string")
            else:
                n_ok = (
                    ("accepts" in n_policy.lower() or "accept" in n_policy.lower())
                    and ("n = 1" in n_policy or "n=1" in n_policy)
                    and ("n = 0" in n_policy or "n=0" in n_policy or "n > 1" in n_policy or "n>1" in n_policy)
                )
                if not n_ok:
                    error(
                        "proxy.n_policy must express: accepts missing n or n = 1, "
                        f"rejects n = 0 or n > 1. Got: {n_policy!r}"
                    )
                else:
                    check("n_policy correct")

        # Disclaimer in description
        desc = cap.get("description", "")
        if "actual upstream model support may vary" not in desc.lower():
            error("capabilities.json description missing disclaimer")
        else:
            check("disclaimer present")


# ── 2. Validate examples/README.md ─────────────────────────────────

print("\n=== 2. examples/README.md ===")

examples_readme = ROOT / "examples" / "README.md"
cargo_toml_path = ROOT / "Cargo.toml"
examples_dir = ROOT / "examples"

if not examples_readme.exists():
    error("examples/README.md not found")
else:
    readme_text = examples_readme.read_text(encoding="utf-8")

    # Extract example names from the table in README
    # Table rows look like: | `demo` | ... |
    listed = set(re.findall(r"\|\s*`([a-z0-9_]+)`\s*\|", readme_text))

    # Filter out non-example entries (like headings, "Example" column header)
    # The table header row starts with "| Example |" - skip that too
    blacklist = {"example"}
    listed = listed - blacklist

    if not listed:
        error("no examples found in examples/README.md table")
    else:
        print(f"  Found {len(listed)} example(s) in README: {sorted(listed)}")

    # Get examples registered in Cargo.toml
    cargo_toml = cargo_toml_path.read_text(encoding="utf-8")
    cargo_examples = set(re.findall(r'\[\[example\]\]\s*\nname\s*=\s*"([^"]+)"', cargo_toml))

    print(f"  Found {len(cargo_examples)} example(s) in Cargo.toml: {sorted(cargo_examples)}")

    # Get .rs files in examples/ dir
    rs_files = {p.stem for p in examples_dir.glob("*.rs")}
    print(f"  Found {len(rs_files)} .rs file(s) in examples/: {sorted(rs_files)}")

    # Every listed example must exist in Cargo.toml and as a .rs file
    for ex in sorted(listed):
        if ex not in cargo_examples:
            error(f"'{ex}' listed in examples/README.md but not in Cargo.toml [[example]]")
        if ex not in rs_files:
            error(f"'{ex}' listed in examples/README.md but examples/{ex}.rs does not exist")

    # Every Cargo.toml example should be listed (optional warning, not error)
    unlisted = cargo_examples - listed
    if unlisted:
        print(f"  NOTE: {sorted(unlisted)} in Cargo.toml but not in examples/README.md (not blocking)")

    # proxy_server command must include --features proxy
    if "proxy_server" in listed:
        # Check that the README mentions --features proxy with proxy_server
        # Look for lines mentioning proxy_server near --features
        features_ok = bool(re.search(
            r"--features\s+proxy.*proxy_server|proxy_server.*--features\s+proxy",
            readme_text
        ))
        if not features_ok:
            error("proxy_server in examples/README.md must include --features proxy in its command")
        else:
            check("proxy_server includes --features proxy")


# ── 3. Scan for known incorrect statements ─────────────────────────

print("\n=== 3. Agent-facing doc phrase scan ===")

DANGEROUS_PHRASES = {
    "subtle crate": "refers to nonexistent subtle crate dependency",
    "Reject n != 1 (or absent)": "incorrect n policy (missing n is accepted)",
    "reject n != 1 (or absent)": "incorrect n policy (missing n is accepted)",
    "all 8 provider implementations": "hardcodes provider count (should not hardcode)",
}

# Context-sensitive check: proxy_server command without --features proxy
PROXY_CMD_NO_FEATURE = re.compile(
    r"cargo\s+run\s+--example\s+proxy_server(?!.*--features\s+proxy)",
    re.IGNORECASE,
)

AGENT_FILES = [
    ROOT / "AGENTS.md",
    ROOT / "docs" / "CONTRACTS.md",
    ROOT / "docs" / "CAPABILITIES.md",
    ROOT / "examples" / "README.md",
    ROOT / "llmrust.capabilities.json",
]

for fpath in AGENT_FILES:
    if not fpath.exists():
        continue
    text = fpath.read_text(encoding="utf-8")
    rel = fpath.relative_to(ROOT)
    clean = True
    for phrase, explanation in DANGEROUS_PHRASES.items():
        if phrase.lower() in text.lower():
            error(f"{rel}: contains '{phrase}' ({explanation})")
            clean = False
    # Context-sensitive: proxy_server without --features proxy
    if PROXY_CMD_NO_FEATURE.search(text):
        error(f"{rel}: proxy_server command without --features proxy")
        clean = False
    if clean:
        check(f"{rel} clean")


# ── 4. Check capability disclaimer ─────────────────────────────────

print("\n=== 4. Capability disclaimer ===")

capabilities_md = ROOT / "docs" / "CAPABILITIES.md"
if capabilities_md.exists():
    text = capabilities_md.read_text(encoding="utf-8")
    if "Actual upstream model support may vary" in text:
        check("docs/CAPABILITIES.md has disclaimer")
    else:
        error("docs/CAPABILITIES.md missing disclaimer: 'Actual upstream model support may vary'")
else:
    error("docs/CAPABILITIES.md not found")


# ── Summary ────────────────────────────────────────────────────────

print(f"\n{'=' * 60}")
if ERRORS == 0:
    print("All agent doc validations passed!")
    sys.exit(0)
else:
    print(f"{ERRORS} error(s) found. Fix them before merging.")
    sys.exit(1)

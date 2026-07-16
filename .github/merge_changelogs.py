import re, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

CHANGELOGS = [
    ROOT / "lux-cli" / "CHANGELOG.md",
    ROOT / "lux-lib" / "CHANGELOG.md",
    ROOT / "lux-macros" / "CHANGELOG.md",
]

OUTPUT = ROOT / "CHANGELOG.md"

ENTRY_RE = re.compile(
    r"^## \[([^\]]+)\]\(([^)]+)\) `([^`]+)` - (\d{4}-\d{2}-\d{2})\n"
    r"(.*?)(?=\n## \[|\Z)",
    re.DOTALL | re.MULTILINE,
)


def main() -> None:
    all_entries = []
    header = None

    for path in CHANGELOGS:
        if not path.exists():
            print(f"warning: {path.name} not found, skipping", file=sys.stderr)
            continue
        text = path.read_text()
        if header is None:
            m = re.search(r"^## \[", text, re.MULTILINE)
            header = text[: m.start()].rstrip() if m else text
        for m in ENTRY_RE.finditer(text):
            all_entries.append((
                m.group(4),  # date
                m.group(3),  # package name
                m.group(1),  # version
                m.group(2),  # url
                m.group(0),  # full entry text
            ))

    if not all_entries:
        print("error: no entries found", file=sys.stderr)
        sys.exit(1)

    # sort by (date, package_name)
    all_entries.sort(key=lambda e: e[:2], reverse=True)

    out = header + "\n\n\n" if header else ""
    for _, _, _, _, body in all_entries:
        out += body + "\n"

    OUTPUT.write_text(out)
    print(f"wrote {len(all_entries)} entries")

if __name__ == "__main__":
    main()

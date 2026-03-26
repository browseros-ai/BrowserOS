import os
import hashlib
from datetime import datetime, timezone


ROOT_DIR = os.path.dirname(os.path.abspath(__file__))
OUTPUT_MD = os.path.join(ROOT_DIR, "all_i_know.md")
SKIP_DIR_NAMES = {
    ".git",
    "node_modules",
    "dist",
    ".wxt",
    ".output",
    ".cache",
    "__pycache__",
}

# If a file is detected as "text-like", we will embed full contents if it's small enough.
MAX_EMBED_TEXT_BYTES = 200_000
# For larger text-like files, we embed a head/tail excerpt.
MAX_HEAD_TAIL_BYTES = 60_000

# Detect binary vs text:
# - If the first chunk contains NUL bytes -> binary
# - Else try UTF-8 decode and check replacement char ratio
TEXT_PROBE_BYTES = 32_768
MAX_REPLACEMENT_RATIO = 0.01

# Hashing every file (including large binaries) can be slow, so only hash small files.
MAX_HASH_BYTES = 5_000_000

LANG_BY_EXT = {
    ".ts": "typescript",
    ".tsx": "tsx",
    ".js": "javascript",
    ".jsx": "jsx",
    ".json": "json",
    ".md": "markdown",
    ".mdx": "mdx",
    ".css": "css",
    ".html": "html",
    ".yml": "yaml",
    ".yaml": "yaml",
    ".toml": "toml",
    ".txt": "",
    ".py": "python",
    ".sql": "sql",
    ".sh": "bash",
    ".ps1": "powershell",
    ".c": "c",
    ".cc": "cpp",
    ".cpp": "cpp",
    ".h": "c",
    ".hpp": "cpp",
    ".go": "go",
    ".rs": "rust",
    ".java": "java",
    ".kt": "kotlin",
    ".xml": "xml",
    ".ini": "ini",
    ".conf": "",
    ".lock": "",
}


def relpath(p: str) -> str:
    rp = os.path.relpath(p, ROOT_DIR)
    return rp.replace("\\", "/")


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def is_text_like(path: str) -> tuple[bool, str]:
    try:
        with open(path, "rb") as f:
            probe = f.read(TEXT_PROBE_BYTES)
        if b"\x00" in probe:
            return False, "binary (NUL byte found in probe)"
        try:
            decoded = probe.decode("utf-8")
        except UnicodeDecodeError:
            # Some text files won't be strict UTF-8; treat as "binary-ish"
            return False, "binary (not valid UTF-8 in probe)"
        # If many replacement characters appear, it's probably not real text.
        replacement_count = decoded.count("\ufffd")
        ratio = replacement_count / max(1, len(decoded))
        if ratio > MAX_REPLACEMENT_RATIO:
            return False, f"binary (UTF-8 replacement ratio too high: {ratio:.4%})"
        return True, "text-like (UTF-8 probe passed)"
    except Exception as e:
        return False, f"binary (probe failed: {e.__class__.__name__})"


def read_text_or_excerpt(path: str, is_text: bool) -> tuple[str, str]:
    """
    Returns (content, note).
    note describes truncation / embedding.
    """
    try:
        size = os.path.getsize(path)
        if not is_text:
            return "", "not embedded (binary file)"

        if size <= MAX_EMBED_TEXT_BYTES:
            with open(path, "rb") as f:
                raw = f.read()
            # decode errors are replaced to keep generation robust
            text = raw.decode("utf-8", errors="replace")
            return text, "embedded (full file)"

        # Excerpt: head + tail bytes
        with open(path, "rb") as f:
            head = f.read(MAX_HEAD_TAIL_BYTES)
            if size <= MAX_HEAD_TAIL_BYTES * 2:
                # Not enough for a distinct tail.
                raw = head
                text = raw.decode("utf-8", errors="replace")
                return text, f"embedded (full file < threshold; size={size} bytes)"
            f.seek(max(0, size - MAX_HEAD_TAIL_BYTES))
            tail = f.read(MAX_HEAD_TAIL_BYTES)

        head_text = head.decode("utf-8", errors="replace")
        tail_text = tail.decode("utf-8", errors="replace")
        # Avoid very large content: keep excerpt sizes controlled.
        content = (
            head_text
            + "\n\n"
            + "/* --- TRUNCATED EXCERPT --- */\n"
            + f"/* total_bytes={size} | showing_head_bytes={len(head)} | showing_tail_bytes={len(tail)} */\n"
            + "/* --- /TRUNCATED EXCERPT --- */\n\n"
            + tail_text
        )
        return content, f"embedded (excerpt: head+tail; size={size} bytes)"
    except Exception as e:
        return "", f"not embedded (failed to read: {e.__class__.__name__})"


def format_dt(epoch_seconds: float) -> str:
    dt = datetime.fromtimestamp(epoch_seconds, tz=timezone.utc)
    return dt.isoformat().replace("+00:00", "Z")


def main() -> None:
    all_files: list[str] = []
    for dirpath, _, filenames in os.walk(ROOT_DIR):
        rel_dir = os.path.relpath(dirpath, ROOT_DIR)
        parts = [] if rel_dir == "." else rel_dir.replace("\\", "/").split("/")
        if any(part in SKIP_DIR_NAMES for part in parts):
            continue
        for name in filenames:
            full = os.path.join(dirpath, name)
            # Avoid self-referential generation.
            if os.path.abspath(full) == os.path.abspath(OUTPUT_MD):
                continue
            all_files.append(full)

    all_files.sort(key=lambda p: relpath(p).lower())

    total_bytes = 0
    text_count = 0
    binary_count = 0
    text_bytes = 0
    binary_bytes = 0

    # Pre-compute summaries first.
    per_file_meta: dict[str, dict] = {}
    for p in all_files:
        st = os.stat(p)
        size = st.st_size
        total_bytes += size
        is_text, why = is_text_like(p)
        if is_text:
            text_count += 1
            text_bytes += size
        else:
            binary_count += 1
            binary_bytes += size
        per_file_meta[p] = {
            "rel": relpath(p),
            "size": size,
            "mtime": st.st_mtime,
            "is_text": is_text,
            "why": why,
        }

    # Group by directory (relative).
    grouped: dict[str, list[str]] = {}
    for p in all_files:
        r = per_file_meta[p]["rel"]
        d = os.path.dirname(r)
        if d == "":
            d = "."
        grouped.setdefault(d, []).append(p)

    generated_at = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")

    lines: list[str] = []
    lines.append("# all_i_know.md")
    lines.append("")
    lines.append(f"- Generated at (UTC): `{generated_at}`")
    lines.append(f"- Repo root: `{relpath(ROOT_DIR)}`")
    lines.append(f"- Total files: `{len(all_files)}`")
    lines.append(f"- Total bytes: `{total_bytes}`")
    lines.append(f"- Text-like files: `{text_count}` (`{text_bytes}` bytes)")
    lines.append(f"- Binary/unknown files: `{binary_count}` (`{binary_bytes}` bytes)")
    lines.append("")
    lines.append("## Fork origin")
    lines.append("")
    lines.append("This working copy is a clone of:")
    lines.append("- [Karthikprasadm/Shimmy-Browser (git URL)](https://github.com/Karthikprasadm/Shimmy-Browser.git)")
    lines.append("")
    lines.append("## AI-powered browser overview")
    lines.append("")
    lines.append("BrowserOS is an AI-agent Chromium fork that runs agents natively in your browser.")
    lines.append("It emphasizes privacy by supporting your own API keys and local/offline models (for chat) via tools like Ollama and LM Studio.")
    lines.append("")
    lines.append("Major AI features (see the docs for end-to-end details):")
    lines.append("- Chat and LLM Hub: `docs/features/llm-chat-hub.mdx`")
    lines.append("- Connect Apps via MCP (Model Context Protocol): `docs/features/connect-mcps.mdx`")
    lines.append("- Workflows (visual graph builder): `docs/features/workflows.mdx`")
    lines.append("- Cowork (browser automation + local filesystem tools): `docs/features/cowork.mdx`")
    lines.append("- Scheduled Tasks (run prompts on a schedule): `docs/features/scheduled-tasks.mdx`")
    lines.append("- Bring your own LLM / local models: `docs/features/bring-your-own-llm.mdx` and `docs/features/local-models.mdx`")
    lines.append("- Ad blocking via uBlock Origin: `docs/features/ad-blocking.mdx`")
    lines.append("")
    lines.append("Privacy / locality themes called out in the docs:")
    lines.append("- Scheduled tasks run on your machine in a hidden browser window; results are saved locally and shown in the UI.")
    lines.append("")
    lines.append("## Generation notes")
    lines.append("")
    lines.append(f"- Text embedding threshold: `{MAX_EMBED_TEXT_BYTES}` bytes (full contents)")
    lines.append(f"- Excerpt threshold for larger text: `{MAX_HEAD_TAIL_BYTES}` bytes head/tail")
    lines.append(f"- Binary detection: NUL byte in probe or UTF-8 decode probe failure")
    lines.append(f"- Hashing: only for files <= `{MAX_HASH_BYTES}` bytes")
    lines.append("")
    lines.append("## Repository contents (grouped by directory)")
    lines.append("")

    for d in sorted(grouped.keys(), key=lambda x: x.lower()):
        lines.append(f"## `{d}`")
        lines.append("")
        files_here = grouped[d]
        files_here.sort(key=lambda p: per_file_meta[p]["rel"].lower())

        for p in files_here:
            meta = per_file_meta[p]
            r = meta["rel"]
            size = meta["size"]
            mtime = format_dt(meta["mtime"])
            is_text = meta["is_text"]
            why = meta["why"]

            lines.append(f"### `{r}`")
            lines.append("")
            lines.append(f"- Size: `{size}` bytes")
            lines.append(f"- Modified (UTC): `{mtime}`")
            lines.append(f"- Detected: `{ 'text-like' if is_text else 'binary/unknown' }`")
            lines.append(f"- Probe reason: `{why}`")

            if size <= MAX_HASH_BYTES:
                try:
                    lines.append(f"- SHA-256: `{sha256_file(p)}`")
                except Exception:
                    lines.append("- SHA-256: `<failed to compute>`")
            else:
                lines.append("- SHA-256: `<skipped (file too large for hashing in generator)>`")

            content, note = read_text_or_excerpt(p, is_text)
            lines.append(f"- Embedding note: `{note}`")
            if is_text and content:
                ext = os.path.splitext(p)[1].lower()
                lang = LANG_BY_EXT.get(ext, "")
                fence = "```"
                if lang:
                    fence = f"```{lang}"
                lines.append("")
                # Use a code fence with a language hint where possible.
                lines.append(f"{fence}")
                # Ensure we don't accidentally close the fence; if it happens, replace only the sequence.
                safe_content = content.replace("```", "``\\`")
                lines.append(safe_content)
                lines.append("```")
                lines.append("")
            else:
                if not is_text:
                    lines.append("")
                    lines.append("> Binary/unknown file: content not embedded in markdown.")
                    lines.append("")

    content = "\n".join(lines).rstrip() + "\n"
    with open(OUTPUT_MD, "w", encoding="utf-8") as f:
        f.write(content)


if __name__ == "__main__":
    main()


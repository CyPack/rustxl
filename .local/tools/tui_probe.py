#!/usr/bin/env python3
"""Drive the `xl` binary inside a pty and capture what it renders.

Why this exists
---------------
`xl` is a full-screen terminal application. Unit tests can check the model, but
they cannot answer "did the user actually see the file?". This harness runs the
real binary against a real pty, feeds it keystrokes, and returns the rendered
text so a test can assert on it.

Two things trip people up and both are handled here:

  * a pty created by `pty.fork()` has no window size, so ratatui draws nothing.
    The size is set explicitly via TIOCSWINSZ.
  * the output is full of escape sequences; `clean()` strips them so assertions
    can look for plain text.

Usage
-----
    from tui_probe import run

    screen = run(["data.csv"])                    # open a file, capture screen
    screen = run([], keys=[b"\\r", b"hello", b"\\r"])  # type into the grid

Both return the visible text of the final frame.
"""

from __future__ import annotations

import fcntl
import os
import pty
import re
import select
import struct
import termios
import time

BINARY = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__)))), "target", "release", "xl")

_ESCAPES = re.compile(
    rb"\x1b\][^\x07]*\x07"      # OSC ... BEL
    rb"|\x1b\[[0-9;?]*[a-zA-Z]"  # CSI
    rb"|\x1b[()][B0]"            # charset selection
    rb"|\x1b[=>]"                # keypad mode
)


def clean(raw: bytes) -> str:
    """Strip terminal escape sequences, leaving the text the user can read.

    Only safe for output that is written once. Ratatui redraws differentially —
    it emits just the cells that changed — so a screen assembled this way is a
    concatenation of fragments, not a picture of the screen. Use `screen()` for
    anything that inspects a frame after a keystroke.
    """
    return _ESCAPES.sub(b"", raw).decode("utf-8", "replace")


def screen(raw: bytes, rows: int = 40, cols: int = 120) -> str:
    """Replay the output through a terminal emulator and return what is on screen.

    This is the honest view: escape sequences are interpreted, cursor moves are
    followed, and differential redraws land in the right cells. Without it, a
    frame like "Gamma (3/3)" changing to "Alpha (1/3)" is emitted as a handful of
    disconnected characters — the letters that happen to match are never re-sent —
    and searching the raw stream for the new text finds nothing.
    """
    try:
        import pyte
    except ImportError as exc:  # pragma: no cover - depends on the environment
        raise RuntimeError("screen() needs pyte: pip install pyte") from exc

    emulator = pyte.Screen(cols, rows)
    stream = pyte.Stream(emulator)
    stream.feed(raw.decode("utf-8", "replace"))
    return "\n".join(emulator.display)


def run(
    args: list[str],
    keys: list[bytes] | None = None,
    *,
    settle: float = 1.5,
    key_delay: float = 0.35,
    rows: int = 40,
    cols: int = 120,
    binary: str = BINARY,
    raw: bool = False,
) -> str:
    """Run `xl` with `args`, optionally send `keys`, return the final screen.

    `settle` is how long to wait for the first frame; `key_delay` how long to
    wait after each keystroke before reading again.

    By default the output is replayed through a terminal emulator, so the result
    is the screen as the user would see it after the last keystroke. Pass
    `raw=True` for the concatenated output stream instead, which is what you want
    when checking that something was printed at all rather than what a frame
    ended up looking like.
    """
    pid, fd = pty.fork()
    if pid == 0:  # child
        os.environ["TERM"] = "xterm-256color"
        os.execv(binary, [os.path.basename(binary), *args])

    # A pty starts at 0x0, which makes ratatui render an empty frame.
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    buffer = bytearray()

    def drain(duration: float) -> None:
        deadline = time.time() + duration
        while time.time() < deadline:
            ready, _, _ = select.select([fd], [], [], 0.1)
            if not ready:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            buffer.extend(chunk)

    drain(settle)

    for key in keys or []:
        try:
            os.write(fd, key)
        except OSError:
            break
        drain(key_delay)

    try:
        os.kill(pid, 9)
        os.waitpid(pid, 0)
    except Exception:
        pass
    os.close(fd)

    output = bytes(buffer)
    return clean(output) if raw else screen(output, rows=rows, cols=cols)


def last_frame(text: str) -> str:
    """Return only the final rendered frame, dropping earlier redraws."""
    # Frames are separated by the cursor-home sequence; after stripping escapes
    # the simplest reliable split is on form feeds if present, otherwise return
    # the whole text (callers usually just search for a substring).
    return text.rsplit("\x0c", 1)[-1]


if __name__ == "__main__":
    import sys

    print(run(sys.argv[1:]))

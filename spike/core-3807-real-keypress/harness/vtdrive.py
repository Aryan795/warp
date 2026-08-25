#!/usr/bin/env python3
"""Drive an interactive shell in a PTY through a minimal VT emulator.

Why an emulator: interactive TUIs (atuin) query the cursor position with
DSR (ESC[6n) and refuse to render if nothing answers. This also lets us dump
the *rendered screen* at each step, which is the evidence we actually care
about ("what would the user see").

Steps: send:<escaped>  wait:<sec>  screen:<label>  line:<label>
"""
import argparse, fcntl, os, pty, select, signal, struct, sys, termios, time


class Screen:
    def __init__(self, rows, cols):
        self.rows, self.cols = rows, cols
        self.buf = [[" "] * cols for _ in range(rows)]
        self.x = self.y = 0
        self.saved = (0, 0)
        self.alt = None
        self.scroll_top, self.scroll_bot = 0, rows - 1

    def _blank_row(self):
        return [" "] * self.cols

    def scroll_up(self, n=1):
        for _ in range(n):
            del self.buf[self.scroll_top]
            self.buf.insert(self.scroll_bot, self._blank_row())

    def put(self, ch):
        if self.x >= self.cols:
            self.x = 0
            self.newline()
        self.buf[self.y][self.x] = ch
        self.x += 1

    def newline(self):
        if self.y == self.scroll_bot:
            self.scroll_up()
        elif self.y < self.rows - 1:
            self.y += 1

    def clamp(self):
        self.x = max(0, min(self.x, self.cols - 1))
        self.y = max(0, min(self.y, self.rows - 1))

    def render(self):
        return "\n".join("".join(r).rstrip() for r in self.buf)

    def cursor_line(self):
        return "".join(self.buf[self.y]).rstrip()


class Term:
    def __init__(self, rows, cols, write):
        self.s = Screen(rows, cols)
        self.write = write
        self.state = "text"
        self.acc = ""

    def feed(self, data):
        for b in data.decode("utf-8", "replace"):
            self.step(b)

    def step(self, ch):
        st = self.state
        if st == "text":
            if ch == "\x1b":
                self.state, self.acc = "esc", ""
            elif ch == "\r":
                self.s.x = 0
            elif ch == "\n":
                self.s.newline()
            elif ch == "\b":
                self.s.x = max(0, self.s.x - 1)
            elif ch == "\t":
                self.s.x = min(self.s.cols - 1, (self.s.x // 8 + 1) * 8)
            elif ch == "\x07":
                pass
            elif ch >= " ":
                self.s.put(ch)
            return
        if st == "esc":
            if ch == "[":
                self.state, self.acc = "csi", ""
            elif ch == "]":
                self.state, self.acc = "osc", ""
            elif ch in "P^_":
                self.state, self.acc = "dcs", ""
            elif ch == "7":
                self.s.saved = (self.s.x, self.s.y); self.state = "text"
            elif ch == "8":
                self.s.x, self.s.y = self.s.saved; self.state = "text"
            elif ch == "M":
                if self.s.y == self.s.scroll_top:
                    self.s.buf.insert(self.s.scroll_top, self.s._blank_row())
                    del self.s.buf[self.s.scroll_bot + 1]
                else:
                    self.s.y -= 1
                self.state = "text"
            else:
                self.state = "text"
            return
        if st == "csi":
            self.acc += ch
            if "@" <= ch <= "~":
                self.csi(self.acc)
                self.state = "text"
            return
        if st in ("osc", "dcs"):
            self.acc += ch
            if ch == "\x07" or self.acc.endswith("\x1b\\") or ch == "\x9c":
                self.state = "text"
            return

    def csi(self, seq):
        final, body = seq[-1], seq[:-1]
        priv = body.startswith("?")
        if priv:
            body = body[1:]
        params = [int(p) if p.isdigit() else 0 for p in body.split(";")] if body else []

        def p(i, d=1):
            return params[i] if i < len(params) and params[i] else d

        s = self.s
        if final == "A": s.y = max(0, s.y - p(0))
        elif final == "B": s.y = min(s.rows - 1, s.y + p(0))
        elif final == "C": s.x = min(s.cols - 1, s.x + p(0))
        elif final == "D": s.x = max(0, s.x - p(0))
        elif final == "E": s.y = min(s.rows - 1, s.y + p(0)); s.x = 0
        elif final == "F": s.y = max(0, s.y - p(0)); s.x = 0
        elif final == "G": s.x = min(s.cols - 1, p(0) - 1)
        elif final == "d": s.y = min(s.rows - 1, p(0) - 1)
        elif final in "Hf": s.y = min(s.rows - 1, p(0) - 1); s.x = min(s.cols - 1, p(1) - 1)
        elif final == "J":
            m = p(0, 0)
            if m == 0:
                s.buf[s.y][s.x:] = [" "] * (s.cols - s.x)
                for r in range(s.y + 1, s.rows): s.buf[r] = s._blank_row()
            elif m == 1:
                s.buf[s.y][: s.x + 1] = [" "] * (s.x + 1)
                for r in range(0, s.y): s.buf[r] = s._blank_row()
            else:
                for r in range(s.rows): s.buf[r] = s._blank_row()
        elif final == "K":
            m = p(0, 0)
            if m == 0: s.buf[s.y][s.x:] = [" "] * (s.cols - s.x)
            elif m == 1: s.buf[s.y][: s.x + 1] = [" "] * (s.x + 1)
            else: s.buf[s.y] = s._blank_row()
        elif final == "L":
            for _ in range(p(0)):
                s.buf.insert(s.y, s._blank_row()); del s.buf[s.scroll_bot + 1]
        elif final == "M":
            for _ in range(p(0)):
                del s.buf[s.y]; s.buf.insert(s.scroll_bot, s._blank_row())
        elif final == "P":
            n = p(0); row = s.buf[s.y]
            del row[s.x : s.x + n]; row.extend([" "] * n)
        elif final == "@":
            n = p(0); row = s.buf[s.y]
            for _ in range(n): row.insert(s.x, " ")
            del row[s.cols :]
        elif final == "X":
            n = p(0); s.buf[s.y][s.x : s.x + n] = [" "] * min(n, s.cols - s.x)
        elif final == "S": s.scroll_up(p(0))
        elif final == "T":
            for _ in range(p(0)):
                s.buf.insert(s.scroll_top, s._blank_row()); del s.buf[s.scroll_bot + 1]
        elif final == "r":
            s.scroll_top = p(0) - 1; s.scroll_bot = min(s.rows - 1, p(1, s.rows) - 1)
        elif final == "n" and not priv:
            if p(0, 0) == 6:
                self.write(f"\x1b[{s.y + 1};{s.x + 1}R".encode())
        elif final in "hl" and priv:
            for v in params:
                if v == 1049:
                    if final == "h" and s.alt is None:
                        s.alt = (s.buf, s.x, s.y)
                        s.buf = [s._blank_row() for _ in range(s.rows)]
                        s.x = s.y = 0
                    elif final == "l" and s.alt is not None:
                        s.buf, s.x, s.y = s.alt; s.alt = None
        s.clamp()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cmd", required=True)
    ap.add_argument("--step", action="append", default=[])
    ap.add_argument("--cols", type=int, default=90)
    ap.add_argument("--rows", type=int, default=24)
    args = ap.parse_args()

    argv = args.cmd.split()
    pid, fd = pty.fork()
    if pid == 0:
        env = dict(os.environ)
        env["TERM"] = "xterm-256color"
        os.execvpe(argv[0], argv, env)
        os._exit(1)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", args.rows, args.cols, 0, 0))

    term = Term(args.rows, args.cols, lambda b: os.write(fd, b))

    def drain(t):
        end = time.time() + t
        while time.time() < end:
            r, _, _ = select.select([fd], [], [], min(end - time.time(), 0.03))
            if r:
                try:
                    d = os.read(fd, 65536)
                except OSError:
                    return
                if not d:
                    return
                term.feed(d)

    drain(1.5)
    for step in args.step:
        kind, _, val = step.partition(":")
        if kind == "send":
            os.write(fd, val.encode("utf-8").decode("unicode_escape").encode("latin-1"))
        elif kind == "wait":
            drain(float(val))
        elif kind == "screen":
            print(f"\n===== SCREEN [{val}] =====")
            print(term.s.render())
            print(f"----- cursor row={term.s.y} col={term.s.x} alt={term.s.alt is not None} -----")
        elif kind == "line":
            print(f"[{val}] CURSOR LINE: {term.s.cursor_line()!r}")
    drain(0.4)
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


main()

#!/usr/bin/env python3
"""Shell-side test matrix for the raw-keypress ctrl-r handoff prototype
(CORE-3807, an alternative to PR #15513's foreground-command handoff).

Drives real, unmodified bash/zsh/fish interactively on a PTY and sources the
actual bundled bootstrap scripts (with `#include` expanded exactly as they
are when Warp injects them into a real session), then exercises the parts of
the wrapper-widget installation and completion-token protocol that a plain
unit test can't reach because they depend on real shell keybinding state:

  - keymap/mode classification: a real third-party ctrl-r widget must be
    recognized (installing a re-invoking wrapper), while a shell builtin
    default must not (installing the empty-completion fallback instead).
    This is what regressed for zsh's stock `redisplay` (viins) and `redo`
    (vicmd) defaults before `$widgets[...]` classification replaced a
    hardcoded name list.
  - the occupied-Alt-] case: a pre-existing user binding on the private key
    sequence must be left untouched -- no wrapper, no fallback -- and the
    session-wide capability tag must be withheld entirely, since a single
    tag can't tell Warp which keymap/mode is actually active.
  - the fallback path: a keymap/mode with no real widget to re-invoke must
    report the pasted token back immediately, with an empty selection.
  - the full completion-token round trip through a real wrapper: the token
    Warp pastes must be echoed back unchanged, and the reported selection
    must be exactly what the wrapped widget produced, with the started hook
    observed first.

Requires: bash, zsh, fish, and python3 on PATH. No third-party dependencies.

Usage: python3 raw_keypress_ctrl_r_matrix.py
Exits 0 if every case passes, 1 otherwise (with a PASS/FAIL line per case).
"""
import json
import os
import pty
import re
import select
import shutil
import sys
import tempfile
import time
from pathlib import Path

BOOTSTRAP_DIR = Path(__file__).resolve().parent.parent

BRACKETED_PASTE_PREFIX = b"\x1b[200~"
BRACKETED_PASTE_SUFFIX = b"\x1b[201~"
# Mirrors RAW_KEYPRESS_CTRL_R_HANDOFF_KEYSEQ in app/src/terminal/view.rs (Alt-]).
HANDOFF_KEYSEQ = b"\x1b]"
PLUGIN_TAG = "external_ctrl_r_raw_keypress"
HOOK_NAME = "ExternalCtrlRRawKeypressSelection"
STARTED_HOOK_NAME = "ExternalCtrlRRawKeypressStarted"
BOOTSTRAPPED_HOOK_NAME = "Bootstrapped"
ALL_HOOK_NAMES = {HOOK_NAME, STARTED_HOOK_NAME, BOOTSTRAPPED_HOOK_NAME}

DCS_HOOK_RE = re.compile(rb"\x1bP\$d([0-9a-fA-F]+)\x1b\\")

results = []


def record(name, ok, detail=""):
    results.append((name, ok, detail))
    status = "PASS" if ok else "FAIL"
    line = f"  {status}  {name}"
    if detail and not ok:
        line += f" -- {detail}"
    print(line)


def assemble_bootstrap(shell):
    """Returns the fully-interpolated bootstrap script for `shell`, i.e. the
    outer <shell>.sh with its `#include bundled/bootstrap/<shell>_body.sh`
    directive replaced by the real body script's contents. Fish has no
    separate body file."""
    if shell == "fish":
        return (BOOTSTRAP_DIR / "fish.sh").read_text()
    outer = (BOOTSTRAP_DIR / f"{shell}.sh").read_text()
    body = (BOOTSTRAP_DIR / f"{shell}_body.sh").read_text()
    lines = []
    for line in outer.split("\n"):
        if line.strip().startswith("#include "):
            lines.append(body)
        else:
            lines.append(line)
    return "\n".join(lines)


class PtySession:
    """A real interactive shell running on its own PTY."""

    def __init__(self, argv, env):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.execvpe(argv[0], argv, env)
            os._exit(1)
        self.buf = b""

    def drain(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], max(0, end - time.time()))
            if not r:
                continue
            try:
                chunk = os.read(self.fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            self.buf += chunk

    def send(self, data, wait=1.0):
        if isinstance(data, str):
            data = data.encode()
        os.write(self.fd, data)
        self.drain(wait)

    def all_hooks(self):
        """Every recognized hook seen so far, as (name, value) tuples in the order
        they appear in the pty stream."""
        out = []
        for m in DCS_HOOK_RE.finditer(self.buf):
            try:
                raw = bytes.fromhex(m.group(1).decode())
                payload = json.loads(raw)
            except (ValueError, UnicodeDecodeError):
                continue
            name = payload.get("hook")
            if name in ALL_HOOK_NAMES:
                out.append((name, payload.get("value")))
        return out

    def hooks(self):
        """Every ExternalCtrlRRawKeypressSelection hook payload seen so far, decoded."""
        return [v for name, v in self.all_hooks() if name == HOOK_NAME]

    def started_hooks(self):
        """Every ExternalCtrlRRawKeypressStarted hook payload seen so far, decoded."""
        return [v for name, v in self.all_hooks() if name == STARTED_HOOK_NAME]

    def bootstrapped_shell_plugins(self):
        """The `shell_plugins` tags from the session's Bootstrapped hook, as a list."""
        for name, v in self.all_hooks():
            if name == BOOTSTRAPPED_HOOK_NAME:
                return [tag for tag in (v or {}).get("shell_plugins", "").split("\n") if tag]
        return []

    def close(self):
        try:
            os.kill(self.pid, 9)
        except ProcessLookupError:
            pass


def send_handoff(session, token, wait=1.5):
    """Writes the exact bytes TerminalView::raw_keypress_ctrl_r_handoff_payload
    writes to the pty: the token wrapped in bracketed-paste markers, followed
    by the private key sequence."""
    session.send(
        BRACKETED_PASTE_PREFIX + token.encode() + BRACKETED_PASTE_SUFFIX + HANDOFF_KEYSEQ,
        wait=wait,
    )


def check_started_then_selection(session, token, label):
    """Asserts the started hook for `token` was observed exactly once, before the
    selection hook for the same token."""
    seq = [name for name, v in session.all_hooks() if (v or {}).get("token") == token]
    ok = seq == [STARTED_HOOK_NAME, HOOK_NAME]
    record(
        f"{label}: started hook observed before selection (token={token})",
        ok,
        detail=str(seq) if not ok else "",
    )


def check_no_started_hook(session, label):
    """Asserts the fallback (immediate-report) path never emits a started hook."""
    record(f"{label}: fallback path does not emit a started hook", len(session.started_hooks()) == 0)


def check_plugin_tag(session, expected_present, label):
    """Asserts whether the session-wide capability tag is present in the Bootstrapped hook."""
    tags = session.bootstrapped_shell_plugins()
    present = PLUGIN_TAG in tags
    ok = present == expected_present
    verb = "advertised" if expected_present else "withheld"
    record(f"{label}: capability tag {verb}", ok, detail=str(tags) if not ok else "")


def check_no_hooks_for_token(session, token, label):
    """Asserts no ExternalCtrlRRawKeypress* hook was ever emitted for `token` -- e.g. because the
    private key sequence invoked an unrelated pre-existing user binding instead of our wrapper or
    fallback in the active keymap."""
    hooks_for_token = [name for name, v in session.all_hooks() if (v or {}).get("token") == token]
    record(
        f"{label}: occupied keymap ignores the handoff sequence (token={token})",
        len(hooks_for_token) == 0,
        detail=str(hooks_for_token) if hooks_for_token else "",
    )


def base_env(home, session_id):
    env = dict(os.environ)
    env["HOME"] = str(home)
    env["WARP_SESSION_ID"] = str(session_id)
    env["WARP_IS_LOCAL_SHELL_SESSION"] = "1"
    env["WARP_USING_WINDOWS_CON_PTY"] = "false"
    env["WARP_HONOR_PS1"] = "0"
    env["TERM"] = "xterm-256color"
    return env


def write_bootstrap_file(tmpdir, shell):
    path = Path(tmpdir) / f"real_{shell}_bootstrap.sh"
    path.write_text(assemble_bootstrap(shell))
    return path


# ---------------------------------------------------------------------------
# bash
# ---------------------------------------------------------------------------


def run_bash_case(name, tmpdir, rc_extra, check):
    home = Path(tmpdir) / f"bash-home-{name}"
    home.mkdir(parents=True, exist_ok=True)
    (home / ".bashrc").write_text(f"""
export PATH={os.environ.get('PATH', '')}
PS1='BASHPROMPT> '
{rc_extra}
""")
    bootstrap_path = write_bootstrap_file(tmpdir, "bash")
    env = base_env(home, 100000)
    env["WARP_IN_MSYS2"] = "false"
    # Production Warp starts bash with a minimal --rcfile (never the user's own
    # ~/.bashrc) and injects the bootstrap script separately; using --rcfile here
    # to load our synthetic rc (simulating a shell where fzf/atuin/etc. have
    # already installed a ctrl-r binding) reproduces that same starting state.
    session = PtySession(["/usr/bin/bash", "--rcfile", str(home / ".bashrc"), "-i"], env)
    try:
        session.drain(1.0)
        session.send(f"source {bootstrap_path}\n", wait=1.5)
        check(session)
    finally:
        session.close()


def test_bash():
    with tempfile.TemporaryDirectory() as tmpdir:
        # A real third-party widget (as fzf/atuin install via `bind -x`) in the emacs
        # keymap must be classified and wrapped, and the full token/selection round
        # trip through it must work.
        def check_real_widget(session):
            session.send(
                "bind -m emacs -X | grep -qF __warp_run_raw_keypress_ctrl_r_widget_emacs "
                "&& echo BASH_EMACS_WRAPPED\n",
                wait=1.0,
            )
            wrapped = b"BASH_EMACS_WRAPPED" in session.buf
            record("bash: real widget classified and wrapped (emacs)", wrapped)

            send_handoff(session, "111")
            hooks = session.hooks()
            ok = len(hooks) == 1 and hooks[0].get("token") == "111" and hooks[0].get(
                "buffer"
            ) == "echo real-widget-selection"
            record(
                "bash: real widget round trip (emacs)",
                ok,
                detail=str(hooks) if not ok else "",
            )
            check_started_then_selection(session, "111", "bash")
            check_plugin_tag(session, True, "bash")

        run_bash_case(
            "real-widget",
            tmpdir,
            'my_ctrl_r_widget() { READLINE_LINE="echo real-widget-selection"; READLINE_POINT=${#READLINE_LINE}; }\n'
            'bind -m emacs -x \'"\\C-r": my_ctrl_r_widget\'\n',
            check_real_widget,
        )

        # vi-command has no `bind -x` ctrl-r binding by default (bash's own
        # reverse-search-history there isn't a `-x` binding), so classification must
        # fail and the fallback must be installed, reporting the token immediately
        # with an empty selection.
        def check_fallback(session):
            session.send(
                'bind -m vi-command -X | grep -qF "\\"\\\\e]\\"" && echo BASH_VICMD_WRAPPED\n',
                wait=1.0,
            )
            wrapped = b"BASH_VICMD_WRAPPED" in session.buf
            record("bash: vi-command keyseq claimed (fallback installed)", wrapped)

            session.send("set -o vi\n", wait=0.5)
            send_handoff(session, "222")
            hooks = session.hooks()
            ok = len(hooks) == 1 and hooks[0].get("token") == "222" and hooks[0].get("buffer") == ""
            record(
                "bash: fallback path reports token with empty selection (vi-command)",
                ok,
                detail=str(hooks) if not ok else "",
            )
            check_no_started_hook(session, "bash")
            check_plugin_tag(session, True, "bash")

        run_bash_case("fallback", tmpdir, "", check_fallback)

        # A pre-existing user binding on Alt-] in vi-insert must be left untouched.
        def check_occupied(session):
            session.send(
                "bind -m vi-insert -X > /tmp/_bash_occ_vi_insert.txt 2>&1\n"
                "bind -m emacs -X > /tmp/_bash_occ_emacs.txt 2>&1\n",
                wait=1.0,
            )
            vi_insert = Path("/tmp/_bash_occ_vi_insert.txt").read_text()
            emacs = Path("/tmp/_bash_occ_emacs.txt").read_text()
            preserved = "my_custom_alt_bracket" in vi_insert
            not_wrapped = (
                "__warp_run_raw_keypress_ctrl_r_widget" not in vi_insert
                and "__warp_report_raw_keypress_ctrl_r_selection" not in vi_insert
            )
            record(
                "bash: occupied Alt-] left untouched (vi-insert)",
                preserved and not_wrapped,
                detail=f"vi-insert binds: {vi_insert!r}" if not (preserved and not_wrapped) else "",
            )
            # An unrelated keymap (emacs) must still get the fallback/wrapper.
            claimed = "__warp_run_raw_keypress_ctrl_r_widget" in emacs or (
                "__warp_report_raw_keypress_ctrl_r_selection" in emacs
            )
            record("bash: unaffected keymap still claims Alt-] (emacs)", claimed)

            # Since one keymap is occupied, the session-wide capability tag must be withheld
            # entirely -- otherwise Warp could send the private sequence into vi-insert while it's
            # active, invoking the user's unrelated binding there instead of a ctrl-r wrapper.
            check_plugin_tag(session, False, "bash")

            # Actually attempt a handoff while vi-insert (the occupied keymap) is active: the
            # private sequence must reach the user's own binding, not report anything to Warp.
            session.send("set -o vi\n", wait=0.5)
            send_handoff(session, "777")
            check_no_hooks_for_token(session, "777", "bash")

        run_bash_case(
            "occupied",
            tmpdir,
            'my_custom_alt_bracket() { READLINE_LINE="CUSTOM"; }\n'
            'bind -m vi-insert -x \'"\\e]": my_custom_alt_bracket\'\n',
            check_occupied,
        )


# ---------------------------------------------------------------------------
# zsh
# ---------------------------------------------------------------------------


def run_zsh_case(name, tmpdir, rc_extra, check):
    home = Path(tmpdir) / f"zsh-home-{name}"
    home.mkdir(parents=True, exist_ok=True)
    (home / ".zshrc").write_text(f"""
export PATH={os.environ.get('PATH', '')}
PROMPT='ZSHPROMPT> '
HISTFILE=$HOME/.zsh_history
{rc_extra}
""")
    bootstrap_path = write_bootstrap_file(tmpdir, "zsh")
    env = base_env(home, 200000)
    env["ZDOTDIR"] = str(home)
    session = PtySession(["/usr/bin/zsh", "-i"], env)
    try:
        session.drain(1.2)
        session.send(f"source {bootstrap_path}\n", wait=1.5)
        check(session)
    finally:
        session.close()


def test_zsh():
    with tempfile.TemporaryDirectory() as tmpdir:
        # A widget genuinely registered via `zle -N` (exactly how fzf/atuin/etc.
        # install themselves) must be classified as real and wrapped.
        def check_real_widget(session):
            session.send(
                "bindkey -M emacs '\\e]' | grep -qF __warp_run_raw_keypress_ctrl_r_widget_emacs "
                "&& echo ZSH_EMACS_WRAPPED\n",
                wait=1.0,
            )
            wrapped = b"ZSH_EMACS_WRAPPED" in session.buf
            record("zsh: user-registered widget classified and wrapped (emacs)", wrapped)

            send_handoff(session, "333")
            hooks = session.hooks()
            ok = len(hooks) == 1 and hooks[0].get("token") == "333" and hooks[0].get(
                "buffer"
            ) == "echo real-widget-selection"
            record(
                "zsh: real widget round trip (emacs)",
                ok,
                detail=str(hooks) if not ok else "",
            )
            check_started_then_selection(session, "333", "zsh")
            check_plugin_tag(session, True, "zsh")

        run_zsh_case(
            "real-widget",
            tmpdir,
            "my_ctrl_r_widget() { BUFFER='echo real-widget-selection'; CURSOR=${#BUFFER}; }\n"
            "zle -N my_ctrl_r_widget\n"
            "bindkey -M emacs '^R' my_ctrl_r_widget\n",
            check_real_widget,
        )

        # Regression coverage for the fix under test: zsh's own stock ctrl-r
        # defaults -- `redisplay` in viins, `redo` in vicmd -- are builtins, not
        # `zle -N`-registered widgets, and must not be classified as a real
        # history tool to hand off to (they used to be, before classification
        # switched from a hardcoded name list to `$widgets[...]`).
        def check_builtin_defaults_rejected(session):
            session.send(
                "print -r -- \"widgets[redisplay]=$widgets[redisplay]\"\n"
                "print -r -- \"widgets[redo]=$widgets[redo]\"\n",
                wait=0.5,
            )
            builtin_classification = b"widgets[redisplay]=builtin" in session.buf and (
                b"widgets[redo]=builtin" in session.buf
            )
            record(
                "zsh: stock ctrl-r defaults classify as builtin, not user widgets",
                builtin_classification,
            )

            session.send(
                "bindkey -M viins '\\e]' | grep -qF __warp_run_raw_keypress_ctrl_r_widget_viins "
                "&& echo VIINS_WRAPPED || echo VIINS_NOT_WRAPPED\n",
                wait=1.0,
            )
            not_wrapped = b"VIINS_NOT_WRAPPED" in session.buf
            record("zsh: viins default (redisplay) does not get a real wrapper", not_wrapped)

            send_handoff(session, "444")
            hooks = session.hooks()
            ok = len(hooks) == 1 and hooks[0].get("token") == "444" and hooks[0].get("buffer") == ""
            record(
                "zsh: fallback path reports token with empty selection (viins default)",
                ok,
                detail=str(hooks) if not ok else "",
            )
            check_no_started_hook(session, "zsh")
            check_plugin_tag(session, True, "zsh")

        run_zsh_case("builtin-defaults", tmpdir, "", check_builtin_defaults_rejected)

        # A pre-existing user binding on Alt-] in emacs must be left untouched.
        def check_occupied(session):
            session.send(
                "bindkey -M emacs '\\e]' > /tmp/_zsh_occ_emacs.txt 2>&1\n"
                "bindkey -M viins '\\e]' > /tmp/_zsh_occ_viins.txt 2>&1\n",
                wait=1.0,
            )
            emacs = Path("/tmp/_zsh_occ_emacs.txt").read_text()
            viins = Path("/tmp/_zsh_occ_viins.txt").read_text()
            preserved = "my_custom_alt_bracket" in emacs
            record(
                "zsh: occupied Alt-] left untouched (emacs)",
                preserved,
                detail=f"emacs binding: {emacs!r}" if not preserved else "",
            )
            claimed = (
                "__warp_run_raw_keypress_ctrl_r_widget_viins" in viins
                or "__warp_report_raw_keypress_ctrl_r_selection_immediate" in viins
            )
            record("zsh: unaffected keymap still claims Alt-] (viins)", claimed)

            # Since one keymap is occupied, the session-wide capability tag must be withheld
            # entirely -- otherwise Warp could send the private sequence into emacs while it's
            # active, invoking the user's unrelated binding there instead of a ctrl-r wrapper.
            check_plugin_tag(session, False, "zsh")

            # Actually attempt a handoff while emacs (the occupied keymap, and zsh's default) is
            # active: the private sequence must reach the user's own binding, not report anything
            # to Warp.
            send_handoff(session, "888")
            check_no_hooks_for_token(session, "888", "zsh")

        run_zsh_case(
            "occupied",
            tmpdir,
            "my_custom_alt_bracket() { BUFFER='CUSTOM' }\n"
            "zle -N my_custom_alt_bracket\n"
            "bindkey -M emacs '\\e]' my_custom_alt_bracket\n",
            check_occupied,
        )


# ---------------------------------------------------------------------------
# fish
# ---------------------------------------------------------------------------


def run_fish_case(name, tmpdir, config_extra, check):
    home = Path(tmpdir) / f"fish-home-{name}"
    (home / ".config" / "fish").mkdir(parents=True, exist_ok=True)
    (home / ".local" / "share" / "fish").mkdir(parents=True, exist_ok=True)
    (home / ".config" / "fish" / "config.fish").write_text(f"""
set -gx PATH {os.environ.get('PATH', '')}
function fish_prompt; echo -n 'FISHPROMPT> '; end
function fish_greeting; end
{config_extra}
""")
    bootstrap_path = write_bootstrap_file(tmpdir, "fish")
    env = base_env(home, 300000)
    session = PtySession(["/usr/bin/fish", "-i"], env)
    try:
        session.drain(1.5)
        session.send(f"source {bootstrap_path}\n", wait=1.5)
        check(session)
    finally:
        session.close()


def test_fish():
    with tempfile.TemporaryDirectory() as tmpdir:
        # A real user-installed ctrl-r binding (not a `bind --preset` default) in
        # default mode must be classified and wrapped.
        def check_real_widget(session):
            session.send(
                "bind -M default \\x1b\\x5d | grep -qF __warp_run_raw_keypress_ctrl_r_widget_default "
                "&& echo FISH_DEFAULT_WRAPPED\n",
                wait=1.0,
            )
            wrapped = b"FISH_DEFAULT_WRAPPED" in session.buf
            record("fish: user-installed widget classified and wrapped (default)", wrapped)

            send_handoff(session, "555")
            hooks = session.hooks()
            ok = len(hooks) == 1 and hooks[0].get("token") == "555" and hooks[0].get(
                "buffer"
            ) == "echo real-widget-selection"
            record(
                "fish: real widget round trip (default)",
                ok,
                detail=str(hooks) if not ok else "",
            )
            check_started_then_selection(session, "555", "fish")
            check_plugin_tag(session, True, "fish")

        run_fish_case(
            "real-widget",
            tmpdir,
            "function my_ctrl_r_widget\n"
            "  commandline -r 'echo real-widget-selection'\n"
            "end\n"
            "bind \\cr my_ctrl_r_widget\n",
            check_real_widget,
        )

        # With no rebinding, default mode's ctrl-r is fish's own preset history
        # search, which must not be classified as a real tool to hand off to.
        def check_fallback(session):
            session.send(
                "bind -M default \\x1b\\x5d | grep -qF __warp_run_raw_keypress_ctrl_r_widget_default "
                "&& echo FISH_WRAPPED || echo FISH_NOT_WRAPPED\n",
                wait=1.0,
            )
            not_wrapped = b"FISH_NOT_WRAPPED" in session.buf
            record("fish: preset ctrl-r default does not get a real wrapper", not_wrapped)

            send_handoff(session, "666")
            hooks = session.hooks()
            ok = len(hooks) == 1 and hooks[0].get("token") == "666" and hooks[0].get("buffer") == ""
            record(
                "fish: fallback path reports token with empty selection (default)",
                ok,
                detail=str(hooks) if not ok else "",
            )
            check_no_started_hook(session, "fish")
            check_plugin_tag(session, True, "fish")

        run_fish_case("fallback", tmpdir, "", check_fallback)

        # A pre-existing user binding on Alt-] in default mode must be untouched.
        def check_occupied(session):
            session.send(
                "bind -M default \\x1b\\x5d > /tmp/_fish_occ_default.txt 2>&1\n",
                wait=1.0,
            )
            default_binds = Path("/tmp/_fish_occ_default.txt").read_text()
            preserved = "my_custom_alt_bracket" in default_binds
            record(
                "fish: occupied Alt-] left untouched (default)",
                preserved,
                detail=f"default binds: {default_binds!r}" if not preserved else "",
            )

            # Since default mode is occupied, the session-wide capability tag must be withheld
            # entirely -- otherwise Warp could send the private sequence into default mode while
            # it's active, invoking the user's unrelated binding there instead of a ctrl-r wrapper.
            check_plugin_tag(session, False, "fish")

            # Actually attempt a handoff in default mode (occupied, and fish's starting mode): the
            # private sequence must reach the user's own binding, not report anything to Warp.
            send_handoff(session, "999")
            check_no_hooks_for_token(session, "999", "fish")

        run_fish_case(
            "occupied",
            tmpdir,
            "function my_custom_alt_bracket\n"
            "  commandline -r 'CUSTOM'\n"
            "end\n"
            "bind \\x1b\\x5d my_custom_alt_bracket\n",
            check_occupied,
        )


def main():
    for shell in ("bash", "zsh", "fish"):
        if not shutil.which(shell):
            print(f"SKIP: {shell} not found on PATH")
            return 1

    print("== bash ==")
    test_bash()
    print("== zsh ==")
    test_zsh()
    print("== fish ==")
    test_fish()

    passed = sum(1 for _, ok, _ in results if ok)
    failed = len(results) - passed
    print(f"\npassed={passed} failed={failed}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())

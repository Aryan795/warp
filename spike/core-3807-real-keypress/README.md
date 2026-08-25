# CORE-3807 spike: real-keypress ctrl-r handoff

Evidence for the alternative to the foreground-command handoff in
[#15513](https://github.com/warpdotdev/warp/pull/15513). Not intended to merge as-is:
this is a spike harness plus the shell-side snippets it validates.

## The claim being tested

The blockers on fish and on bash+atuin are consequences of invoking the history
widget as a plain foreground command, not properties of the tools. If the widget
runs from a real key-binding context instead, they go away.

## Mechanism

Warp's bootstrap captures whatever the user has bound to `^R`, then installs a
wrapper widget on a private key sequence. Pressing ctrl-r in Warp writes that
sequence to the PTY rather than a bare `^R`. The wrapper:

1. invokes the user's own binding, inside a genuine key-binding context, so
   `commandline` (fish) and `READLINE_LINE` (bash) work;
2. reads the resulting line buffer and reports it over a DCS hook;
3. clears the shell's buffer, so Warp's input editor is the only holder of the
   command.

This beats sending a bare `^R` and scraping the screen: the selection is exact,
the DCS gives a deterministic signal for leaving raw-forward mode, and the shell
is left at a clean prompt.

## Running

Needs `fzf` and `atuin` on `PATH`, plus `python3`. Override `FZF_SHELL` and
`BASH_PREEXEC` to point at fzf's `shell/` directory and `bash-preexec.sh`.

```sh
./run-matrix.sh
```

24 assertions: 3 shells x 2 tools x {select, cancel}, each checking both the
captured selection and that the shell's buffer was left empty.

## Why the harness emulates a terminal

`harness/vtdrive.py` drives an interactive shell on a PTY and writes raw key
bytes to it. It carries a small VT emulator because atuin queries the cursor
position with DSR (`ESC[6n`) and aborts with "The cursor position could not be
read within a normal duration" if nothing answers. Emulating the screen also
makes the rendered result directly inspectable, which is the actual evidence.

```sh
python3 harness/vtdrive.py --cmd '/usr/bin/fish -i' \
  --step 'wait:1' --step 'send:\x12' --step 'wait:2' --step 'screen:widget-open'
```

## Caveats found while testing

- **atuin's default `enter_accept = true` executes the selection** instead of
  returning it. The matrix pins `enter_accept = false`. Any shipped design needs
  a story for users on the default.
- **Each shell uses a different key notation**: bash `\C-x\C-r`, zsh `^X^R`,
  fish literal bytes. Getting fish wrong fails silently — the trailing `^R` fires
  the user's binding directly and the wrapper never runs.
- **`\C-x\C-r` is already bound** to `re-read-init-file` in stock readline, so it
  is a poor choice for the private sequence; pick something unbound.
- **fzf still installs a readline *macro* on bash < 4** rather than a `bind -x`
  function. A macro cannot be invoked from a wrapper, so that configuration needs
  the bare-`^R` fallback.

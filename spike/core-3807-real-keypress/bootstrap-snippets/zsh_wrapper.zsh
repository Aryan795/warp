# Spike artifact for CORE-3807 (real-keypress handoff). Would live in
# app/assets/bundled/bootstrap/zsh_body.sh, sourced after the user's rc files so
# that `bindkey '^R'` reports whatever the user actually configured.
#
# Warp writes WARP_EXTERNAL_CTRL_R_KEYSEQ to the PTY instead of a bare ^R. That
# runs this wrapper inside a genuine zle context, which is the property the
# tools' own widgets depend on, while still letting Warp recover the selection
# out-of-band and leave the shell's line buffer empty.

# Whatever ^R resolves to after the user's rc has run. `bindkey '^R'` prints
# `"^R" widget-name`; (z) splits it into shell words.
__warp_orig_ctrl_r_widget=${${(z)$(bindkey '^R')}[2]}

# Reports the selection to Warp over a DCS hook. Invisible to the terminal
# display, so it is safe to emit from inside a widget.
__warp_report_ctrl_r_selection() {
  printf '\eP$f{"hook": "ExternalCtrlRSelection", "value": {"command": %s}}\x9c' \
    "$(__warp_json_string "$1")" > /dev/tty
}

warp-external-ctrl-r() {
  # `zle <widget>` is the whole point: the tool's widget runs with a live line
  # editor, so zle builtins (vi-fetch-history, redisplay, ...) behave normally.
  zle "$__warp_orig_ctrl_r_widget"

  __warp_report_ctrl_r_selection "$BUFFER"

  # Warp owns the command from here; the shell's own buffer must not keep a copy,
  # or the next Enter would submit it twice.
  BUFFER=''
  CURSOR=0
  zle reset-prompt
}

zle -N warp-external-ctrl-r
bindkey "$WARP_EXTERNAL_CTRL_R_KEYSEQ" warp-external-ctrl-r

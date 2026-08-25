# Spike artifact for CORE-3807 (real-keypress handoff). Would live in
# app/assets/bundled/bootstrap/bash_body.sh, sourced after the user's rc files.
#
# atuin binds ^R with `bind -x` and returns its result by assigning READLINE_LINE,
# which only exists while readline is executing a key binding. Running the widget
# as a plain foreground command therefore cannot work; running it from this
# wrapper, which is itself a `bind -x` binding, gives it exactly the context it
# needs.

# Classifies the user's ^R binding. Only the `bind -x` (funcexec) form can be
# re-invoked from a wrapper: a readline macro is a canned keystroke sequence that
# readline replays, and a plain readline function name is not a shell command.
# fzf still installs the macro form on bash < 4, so the unsupported cases are
# reachable and the client must fall back to sending a bare ^R for them.
__warp_classify_ctrl_r_binding() {
  local line
  line=$(bind -X 2>/dev/null | grep -F '"\C-r"' | head -1)
  if [[ -n $line ]]; then
    line=${line#*: }
    line=${line#\"}
    line=${line%\"}
    __warp_orig_ctrl_r=$line
    __warp_orig_ctrl_r_kind=funcexec
    return
  fi

  if bind -s 2>/dev/null | grep -qF '"\C-r"'; then
    __warp_orig_ctrl_r_kind=macro
    return
  fi

  __warp_orig_ctrl_r_kind=readline
}

__warp_report_ctrl_r_selection() {
  printf '\eP$f{"hook": "ExternalCtrlRSelection", "value": {"command": %s}}\x9c' \
    "$(__warp_json_string "$1")" > /dev/tty
}

__warp_external_ctrl_r() {
  # READLINE_LINE/READLINE_POINT are live globals for the duration of this
  # binding, so the callee's assignments are visible here and readline picks up
  # ours when we return.
  case $__warp_orig_ctrl_r_kind in
    funcexec) eval "$__warp_orig_ctrl_r" ;;
    *) return ;;
  esac

  __warp_report_ctrl_r_selection "$READLINE_LINE"

  READLINE_LINE=''
  READLINE_POINT=0
}

__warp_classify_ctrl_r_binding
if [[ $__warp_orig_ctrl_r_kind == funcexec ]]; then
  bind -x "\"$WARP_EXTERNAL_CTRL_R_KEYSEQ\": __warp_external_ctrl_r"
fi

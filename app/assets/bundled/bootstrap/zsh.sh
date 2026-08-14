 setopt interactivecomments
 # Set PS2 to an empty string to prevent zsh from printing a secondary prompt
 # (e.g.: 'heredoc> '), which would otherwise be printed repeatedly when we
 # paste the bootstrap script into the PTY.
 if (( ${+PS2} )); then
   ORIGINAL_PS2="$PS2"
 fi
 PS2=""
 # Similar to our approach in bash, we start a shell with the minimal amount of
 # startup (i.e. --no-rcs) and then take over by executing the shell startup.
 # We only support the local zsh case for now.
 #
 # Note that we indent everything in this top-level script by one space in order
 # to hide from history.
 #
 # Also, note that we put the 'eval' on the same line as the 'read' separated by a semi-colon
 # rather than on its own line after the HEREDOC.  This seems to work around a bug in zsh
 # where the buffer was getting messed up after processing the heredoc about 1/50 of the time.
 #
 # We restore the tty to sane mode (undoing the 'stty raw' set in zsh_init_shell.sh) right
 # after reading the heredoc but before evaluating it, mirroring our approach in bash. This
 # keeps the tty in raw mode for the entire duration of the paste, so a not-yet-consumed
 # portion of the bootstrap script can never be echoed back to the screen or picked up as
 # typeahead by the shell's line editor, even if the user's rcfiles reconfigure ZLE widgets.
read -r -d '' WARP_BOOTSTRAP_VAR << 'EOM'; stty sane; eval "$WARP_BOOTSTRAP_VAR"; unset WARP_BOOTSTRAP_VAR
#include bundled/bootstrap/zsh_body.sh
EOM

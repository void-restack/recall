use std::path::Path;

const ZSH: &str = r#"# recall shell integration (zsh)
# enable with:  eval "$(recall init zsh)"
_recall_last_file='__LAST_FILE__'
_recall_record_last() {
  case "$1" in
    recall|recall\ *) ;;
    *)
      mkdir -p "${_recall_last_file:h}" 2>/dev/null
      print -r -- "$1" >| "$_recall_last_file"
      ;;
  esac
}
autoload -Uz add-zsh-hook
add-zsh-hook preexec _recall_record_last
"#;

// Alt+R inserts a recalled command at the cursor (never executes it); Alt+S opens
// the capture form for the command you just ran. Cancelling either leaves the line
// untouched.
const ZSH_KEYS: &str = r#"
recall-recall-widget() {
  local selected
  selected="$(recall </dev/tty)"
  [[ -n "$selected" ]] && LBUFFER="${LBUFFER}${selected}"
  zle reset-prompt
}
zle -N recall-recall-widget
bindkey '^[r' recall-recall-widget

recall-save-widget() {
  recall add --last </dev/tty >/dev/tty 2>&1
  zle reset-prompt
}
zle -N recall-save-widget
bindkey '^[s' recall-save-widget
"#;

const BASH: &str = r#"# recall shell integration (bash)
# enable with:  eval "$(recall init bash)"
_recall_last_file='__LAST_FILE__'
_recall_record_last() {
  local last
  last=$(history 1 | sed 's/^ *[0-9]* *//')
  case "$last" in
    recall|recall\ *) return ;;
  esac
  mkdir -p "$(dirname "$_recall_last_file")" 2>/dev/null
  printf '%s\n' "$last" > "$_recall_last_file"
}
case "$PROMPT_COMMAND" in
  *_recall_record_last*) ;;
  *) PROMPT_COMMAND="_recall_record_last${PROMPT_COMMAND:+;$PROMPT_COMMAND}" ;;
esac
"#;

const BASH_KEYS: &str = r#"
recall-recall-widget() {
  local selected
  selected="$(recall </dev/tty)"
  READLINE_LINE="${READLINE_LINE:0:$READLINE_POINT}${selected}${READLINE_LINE:$READLINE_POINT}"
  READLINE_POINT=$(( READLINE_POINT + ${#selected} ))
}
bind -x '"\er": recall-recall-widget'

recall-save-widget() {
  recall add --last </dev/tty >/dev/tty 2>&1
}
bind -x '"\es": recall-save-widget'
"#;

pub fn zsh(last_file: &Path, keys: bool) -> String {
    let mut script = ZSH.replace("__LAST_FILE__", &last_file.to_string_lossy());
    if keys {
        script.push_str(ZSH_KEYS);
    }
    script
}

pub fn bash(last_file: &Path, keys: bool) -> String {
    let mut script = BASH.replace("__LAST_FILE__", &last_file.to_string_lossy());
    if keys {
        script.push_str(BASH_KEYS);
    }
    script
}

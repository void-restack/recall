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
// the capture form for the command you just ran. `_recall_bindkey` refuses to steal
// a key that is already bound to something else, reporting instead of overriding.
const ZSH_KEYS: &str = r#"
recall-recall-widget() {
  emulate -L zsh
  zle -I                       # release the line editor before the full-screen child
  local selected
  # recall draws its UI on stderr (the terminal) and prints the chosen command on stdout,
  # captured here. Its cursor probe is handled internally, so no fd swap is needed.
  selected="$(recall </dev/tty)"
  zle reset-prompt
  [[ -n "$selected" ]] && LBUFFER="${LBUFFER}${selected}"
}
zle -N recall-recall-widget

recall-save-widget() {
  emulate -L zsh
  zle -I
  recall add --last </dev/tty >/dev/tty 2>/dev/tty
  zle reset-prompt
}
zle -N recall-save-widget

_recall_bindkey() {
  local existing="${$(bindkey -- "$1")##* }"
  if [[ -z "$existing" || "$existing" == "undefined-key" || "$existing" == "$2" ]]; then
    bindkey "$1" "$2"
  else
    print -u2 "recall: $1 is already bound ($existing); not overriding — choose another with 'recall init zsh --keys --recall-key ...'"
  fi
}
_recall_bindkey '__RECALL_KEY__' recall-recall-widget
_recall_bindkey '__SAVE_KEY__' recall-save-widget
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
  # UI on stderr (the terminal), chosen command on stdout, captured here. No fd swap:
  # recall handles its own cursor probe. readline redraws the line on return.
  selected="$(recall </dev/tty)"
  READLINE_LINE="${READLINE_LINE:0:$READLINE_POINT}${selected}${READLINE_LINE:$READLINE_POINT}"
  READLINE_POINT=$(( READLINE_POINT + ${#selected} ))
}
bind -x '"__RECALL_KEY__": recall-recall-widget'

recall-save-widget() {
  recall add --last </dev/tty >/dev/tty 2>/dev/tty
}
bind -x '"__SAVE_KEY__": recall-save-widget'
"#;

pub fn zsh(last_file: &Path, keys: bool, recall_key: &str, save_key: &str) -> String {
    let mut script = ZSH.replace("__LAST_FILE__", &last_file.to_string_lossy());
    if keys {
        script.push_str(
            &ZSH_KEYS
                .replace("__RECALL_KEY__", recall_key)
                .replace("__SAVE_KEY__", save_key),
        );
    }
    script
}

pub fn bash(last_file: &Path, keys: bool, recall_key: &str, save_key: &str) -> String {
    let mut script = BASH.replace("__LAST_FILE__", &last_file.to_string_lossy());
    if keys {
        script.push_str(
            &BASH_KEYS
                .replace("__RECALL_KEY__", recall_key)
                .replace("__SAVE_KEY__", save_key),
        );
    }
    script
}

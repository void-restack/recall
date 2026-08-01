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

pub fn zsh(last_file: &Path) -> String {
    ZSH.replace("__LAST_FILE__", &last_file.to_string_lossy())
}

pub fn bash(last_file: &Path) -> String {
    BASH.replace("__LAST_FILE__", &last_file.to_string_lossy())
}

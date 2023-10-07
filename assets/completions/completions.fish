function __fish_complete_inputs
    fe complete inputs
end

function __mycommand_complete
    set -l current_word (commandline -t)
    set -l completions "$current_word hello hello $current_word"
    # echo $completions
    printf '%s%s' "$current_word" "$current_word"
end

complete -c mycommand --no-files --arguments "(__mycommand_complete)"
# complete -c mycommand --no-files --arguments "(bash test.sh)"

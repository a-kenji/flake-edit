function __fish_complete_inputs
    fe complete inputs
end

function __mycommand_complete
    set -l current_word (commandline -t)
    set -l completions "$current_word hello hello $current_word"
    # echo $completions
    printf '%s%s' "$current_word" "$current_word"
end

function __fish_complete_inputs
    fe list 2>/dev/null
end

complete -c mycommand --no-files --arguments "(__mycommand_complete)"
# complete -c mycommand --no-files --arguments "(bash test.sh)"

# function _fe_add
#
# end

complete -c fe -n "__fish_seen_subcommand_from rm" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from remove" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from change" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from c" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from pin" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from p" -f -a "(__fish_complete_inputs)" -d Input
# complete -c fe \
#     --no-files \
#     -s remove \
#     -s rm \
#     -d "Remove a flake input." \
#     --arguments "(__fish_complete_inputs)"

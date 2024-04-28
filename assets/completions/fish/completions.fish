function __fish_complete_inputs
    fe list --format simple 2>/dev/null
end

function __fish_complete_add
    fe completion add 2>/dev/null
end

complete -c fe -n "__fish_seen_subcommand_from rm" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from remove" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from change" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from c" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from pin" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from p" -f -a "(__fish_complete_inputs)" -d Input

set -x ignore_space true
complete -c fe -n "__fish_seen_subcommand_from a" -f -r --keep-order -a  "(__fish_complete_add)" -d Add
complete -c fe -n "__fish_seen_subcommand_from add" -f -r --keep-order -a  "(__fish_complete_add)" -d Add
set -x ignore_space false

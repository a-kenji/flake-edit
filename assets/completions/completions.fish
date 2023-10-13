function __fish_complete_inputs
    fe list --format simple 2>/dev/null
end

complete -c fe -n "__fish_seen_subcommand_from rm" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from remove" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from change" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from c" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from pin" -f -a "(__fish_complete_inputs)" -d Input
complete -c fe -n "__fish_seen_subcommand_from p" -f -a "(__fish_complete_inputs)" -d Input

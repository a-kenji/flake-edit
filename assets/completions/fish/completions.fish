function __fish_complete_inputs
    flake-edit list --format simple 2>/dev/null
end

function __fish_complete_inputs_toplevel
    flake-edit list --format toplevel 2>/dev/null
end

function __fish_complete_add
    flake-edit completion add 2>/dev/null
end

function __fish_complete_follow
    flake-edit completion follow 2>/dev/null
end

complete -c flake-edit -n "__fish_seen_subcommand_from rm" -f -a "(__fish_complete_inputs)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from remove" -f -a "(__fish_complete_inputs)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from change" -f -a "(__fish_complete_inputs)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from c" -f -a "(__fish_complete_inputs)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from pin" -f -a "(__fish_complete_inputs_toplevel)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from p" -f -a "(__fish_complete_inputs_toplevel)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from unpin" -f -a "(__fish_complete_inputs_toplevel)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from up" -f -a "(__fish_complete_inputs_toplevel)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from update" -f -a "(__fish_complete_inputs_toplevel)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from u" -f -a "(__fish_complete_inputs_toplevel)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from follow" -f -a "(__fish_complete_follow)" -d Input
complete -c flake-edit -n "__fish_seen_subcommand_from f" -f -a "(__fish_complete_follow)" -d Input

set -x ignore_space true
complete -c flake-edit -n "__fish_seen_subcommand_from a" -f -r --keep-order -a "(__fish_complete_add)" -d Add
complete -c flake-edit -n "__fish_seen_subcommand_from add" -f -r --keep-order -a "(__fish_complete_add)" -d Add
set -x ignore_space false

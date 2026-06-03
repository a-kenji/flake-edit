{
  inputs = {
    keep.url = "github:owner/keep"; # keep me here
    drop.url = "github:owner/drop"; # drop with me
    after.url = "github:owner/after";
  };
  outputs = { self, ... }: { };
}

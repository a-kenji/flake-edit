{ lib, ... }: {
  homepage = "https://github.com/a-kenji/fe";
  # inherit description;
  mainProgram = "fe";
  license = [ lib.licenses.mit ];
}

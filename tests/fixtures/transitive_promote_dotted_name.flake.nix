{
  description = "Two inputs that share a nested hls-1.10 and a nested zz.";

  inputs = {
    a.url = "github:o/a";
    b.url = "github:o/b";
  };

  outputs = _: { };
}

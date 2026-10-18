{
  description = "Two inputs whose own haskellNix each pull hls-1.10 and hls-2.0.";

  inputs = {
    cardano-faucet.url = "github:input-output-hk/cardano-faucet/node-8.3";
    cardano-node.url = "github:input-output-hk/cardano-node/8.5.0-pre";
  };

  outputs = _: { };
}

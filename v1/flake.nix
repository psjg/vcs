{
  description = "v1 — formal toolchain: everything FORMAL.md and lean/ need to be checked";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      devShells = forAll (pkgs:
        let
          py = pkgs.python3.withPackages (p: [ p.hypothesis p.z3-solver p.pypdf ]);
          agdaC = pkgs.agda.withPackages (p: [ p.standard-library p.cubical ]);
          core = [
            pkgs.elan            # Lean 4 toolchain manager; `lake` for the Mathlib variant
            py                   # Hypothesis proptests (lean/test_worlds.py), z3 bindings
            pkgs.z3 pkgs.cvc5    # SMT, standalone
          ];
          protocol = [
            pkgs.tlaplus         # TLC model checker: sync, redaction, equivocation (tla/)
            pkgs.alloy6          # bounded relational checking: §6 capabilities, 7.6 (alloy/)
          ];
          homotopy = [
            agdaC                # cubical Agda: §C6, 5.6 as a 2-cell (agda/)
          ];
          spikes = [
            pkgs.ats2            # "implementation = model" spike: proofs in the implementation
            pkgs.dafny           # same question with SMT automation
            pkgs.idris2          # QTT reference for FPL's type system
          ];
          crosscheck = [
            pkgs.coq             # Rocq/Coq: cross-check of lean/, Singh's Dilworth
            pkgs.why3
          ];
        in {
          # everything; large (Agda + cubical is ~3 GB unpacked)
          default = pkgs.mkShell {
            packages = core ++ protocol ++ homotopy ++ spikes ++ crosscheck;
            shellHook = ''
              export LEAN_PATH=$PWD/lean
              echo "v1 formal shell — lean (via elan), tlc, alloy, agda (cubical), coq, ats2, dafny, idris2, z3, cvc5, hypothesis"
            '';
          };
          # the three shells you actually sit in
          lean     = pkgs.mkShell { packages = core; };
          protocol = pkgs.mkShell { packages = core ++ protocol; };
          homotopy = pkgs.mkShell { packages = core ++ homotopy; };
          spikes   = pkgs.mkShell { packages = core ++ spikes; };
        });

      # `nix run .#check` — re-check every artefact that has a headless checker
      apps = forAll (pkgs: {
        check = {
          type = "app";
          program = toString (pkgs.writeShellScript "v1-check" ''
            set -e
            cd lean && for f in Worlds Components Fugue Conflicts Holes Moves Projection Authority Contraction; do
              printf '%-12s ' $f; LEAN_PATH=. lean $f.lean >/dev/null && echo ok
            done && cd ..
            python3 lean/test_worlds.py
            ${pkgs.tlaplus}/bin/tlc -workers auto -config tla/Sync.cfg tla/Sync.tla | tail -3
            ${pkgs.alloy6}/bin/alloy exec alloy/Capabilities.als | tail -5
            ${pkgs.alloy6}/bin/alloy exec alloy/Contraction.als | tail -5
            agda agda/Horn.agda
          '');
        };
      });
    };
}

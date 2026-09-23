# v1/theory — one literate file

`THEORY.org` is the source of truth: the theory in prose, and every
definition, theorem, model and build file as tangled code blocks. Nothing
else in this directory is edited by hand. The tangled tree (Lean project,
TLA+, Alloy, Agda, flake, Makefile) is not committed: it is derived, and a
derived copy is a second place for drift.

Bootstrap, from a directory holding only `THEORY.org`:

    nix shell nixpkgs#emacs-nox --command \
      emacs --batch -l org --eval '(org-babel-tangle-file "THEORY.org")'
    nix develop
    make lean        # Mathlib cache for the imports, then lake build
    make check       # tla, alloy, agda as well

`refs/` holds the Dilworth formalisations consulted (Singh, Coq; Maadoori
et al., Isabelle/AFP). `results/` holds model-checker output as evidence.
Supersedes the former `v1/lean`, `v1/tla`, `v1/alloy`, `v1/agda`,
`v1/flake.nix`; `v1/spikes` stays as a record of the Dafny and ATS spikes.

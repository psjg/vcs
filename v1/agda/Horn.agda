{-# OPTIONS --cubical --safe --guardedness #-}
-- v1/FORMAL.md §C6 and 5.6 — the homotopical reading, in cubical Agda.
-- Worlds are points of a type; patches are paths; patch laws are 2-paths.
-- A merge situation is a horn (two patches out of one world); a merge is a
-- filler. In a groupoid every horn fills (Pijul's freely-completed category);
-- "Agree" between two patches is a 2-cell between their images under M.
module Horn where

open import Cubical.Foundations.Prelude
open import Cubical.Foundations.GroupoidLaws
open import Cubical.Data.Sigma

private variable
  ℓ : Level
  W Doc : Type ℓ

-- a horn out of a, and what it means to fill it
Filler : {a b c : W} (p : a ≡ b) (q : a ≡ c) → Type _
Filler {W = W} {b = b} {c = c} p q =
  Σ[ d ∈ W ] Σ[ r ∈ b ≡ d ] Σ[ s ∈ c ≡ d ] (p ∙ r ≡ q ∙ s)

-- every horn fills when patches are invertible: rebase q onto b via p⁻¹
fillHorn : {a b c : W} (p : a ≡ b) (q : a ≡ c) → Filler p q
fillHorn {c = c} p q = c , (sym p ∙ q) , refl , square
  where
    square : p ∙ (sym p ∙ q) ≡ q ∙ refl
    square =
      p ∙ (sym p ∙ q)   ≡⟨ assoc p (sym p) q ⟩
      (p ∙ sym p) ∙ q   ≡⟨ cong (_∙ q) (rCancel p) ⟩
      refl ∙ q          ≡⟨ sym (lUnit q) ⟩
      q                 ≡⟨ rUnit q ⟩
      q ∙ refl          ∎

-- rebase / transport (FORMAL: cherry-pick as transport): a change c at a,
-- carried along p : a ≡ b, is a change at b
rebase : {a a′ b : W} (c : a ≡ a′) (p : a ≡ b) → b ≡ a′
rebase c p = sym p ∙ c

-- 5.6: two patches between the same worlds agree under a materialiser M
-- when their images are equal as paths of documents — a 2-cell in Doc
Agree : (M : W → Doc) {a b : W} (p q : a ≡ b) → Type _
Agree M p q = cong M p ≡ cong M q

agree-refl : (M : W → Doc) {a b : W} (p : a ≡ b) → Agree M p p
agree-refl M p = refl

agree-sym : (M : W → Doc) {a b : W} (p q : a ≡ b) → Agree M p q → Agree M q p
agree-sym M p q h = sym h

agree-trans : (M : W → Doc) {a b : W} (p q r : a ≡ b)
            → Agree M p q → Agree M q r → Agree M p r
agree-trans M p q r h k = h ∙ k

-- and a 2-cell between the patches themselves is the strongest form of
-- agreement: it is preserved by every M (functoriality, ADR-0004)
agree-of-2cell : (M : W → Doc) {a b : W} (p q : a ≡ b) → p ≡ q → Agree M p q
agree-of-2cell M p q h = cong (cong M) h

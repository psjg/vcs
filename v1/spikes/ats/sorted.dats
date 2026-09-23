(* Spike: "implementation = model" in ATS2.
   The weave's sorted insertion, with sortedness carried as a proof
   in the type of the result. Keys are ints (the (hlc, id) tie-break). *)
#include "share/atspre_staload.hats"
staload "libats/SATS/ilist_prf.sats"

(* a list of ints indexed by its static contents and its length *)
datatype slist (ilist, int) =
  | slist_nil (ilist_nil, 0)
  | {x:int}{xs:ilist}{n:nat}
    slist_cons (ilist_cons (x, xs), n+1) of (int x, slist (xs, n))

(* a is a lower bound of xs (with a depth index for termination metrics) *)
dataprop LB (int, ilist, int) =
  | {a:int} LBnil (a, ilist_nil, 0)
  | {a,x:int | a <= x}{xs:ilist}{n:nat}
    LBcons (a, ilist_cons (x, xs), n+1) of LB (a, xs, n)

(* xs is sorted *)
dataprop ORD (ilist) =
  | ORDnil (ilist_nil)
  | {x:int}{xs:ilist}{n:nat}
    ORDcons (ilist_cons (x, xs)) of (LB (x, xs, n), ORD xs)

(* ys is xs with x inserted somewhere *)
dataprop INSERT (int, ilist, ilist, int) =
  | {x:int}{xs:ilist} INSERTbas (x, xs, ilist_cons (x, xs), 0)
  | {x,y:int}{ys,zs:ilist}{n:nat}
    INSERTind (x, ilist_cons (y, ys), ilist_cons (y, zs), n+1) of INSERT (x, ys, zs, n)

prfun lb_weaken {a,b:int | a <= b}{xs:ilist}{n:nat} .<n>.
  (pf: LB (b, xs, n)): LB (a, xs, n) =
  case+ pf of
  | LBnil () => LBnil ()
  | LBcons (pf1) => LBcons (lb_weaken (pf1))

prfun lb_insert {a,x:int | a <= x}{xs,ys:ilist}{n,m:nat} .<m>.
  (pf1: LB (a, xs, n), pf2: INSERT (x, xs, ys, m)): [k:nat] LB (a, ys, k) =
  case+ pf2 of
  | INSERTbas () => LBcons (pf1)
  | INSERTind (pf2') => let
      prval LBcons (pf1') = pf1
      prval pf3 = lb_insert (pf1', pf2')
    in LBcons (pf3) end

(* the executable insertion; the proof of sortedness comes out with the list *)
fun insert {x:int}{xs:ilist}{n:nat} .<n>.
  (pf: ORD xs | x: int x, xs: slist (xs, n))
  : [ys:ilist] [m:nat] (ORD ys, INSERT (x, xs, ys, m) | slist (ys, n+1)) =
  case+ xs of
  | slist_nil () =>
      (ORDcons (LBnil (), ORDnil ()), INSERTbas () | slist_cons (x, slist_nil ()))
  | slist_cons {y}{ys} (y, ys) =>
      if x <= y then let
        prval ORDcons (pf_lb, pf_ord) = pf
        prval pf_lb' = lb_weaken {x, y} (pf_lb)
      in
        (ORDcons (LBcons (pf_lb'), ORDcons (pf_lb, pf_ord)), INSERTbas () | slist_cons (x, xs))
      end else let
        prval ORDcons (pf_lb, pf_ord) = pf
        val (pf_ord2, pf_ins | zs) = insert (pf_ord | x, ys)
        prval pf_lb2 = lb_insert (pf_lb, pf_ins)
      in
        (ORDcons (pf_lb2, pf_ord2), INSERTind (pf_ins) | slist_cons (y, zs))
      end

fun print_slist {xs:ilist}{n:nat} .<n>. (xs: slist (xs, n)): void =
  case+ xs of
  | slist_nil () => println! ()
  | slist_cons (x, xs) => (print! (x, " "); print_slist xs)

implement main0 () = let
  val (pf1, _ | l1) = insert (ORDnil () | 5, slist_nil ())
  val (pf2, _ | l2) = insert (pf1 | 2, l1)
  val (pf3, _ | l3) = insert (pf2 | 9, l2)
  val (pf4, _ | l4) = insert (pf3 | 4, l3)
in
  print_slist l4
end

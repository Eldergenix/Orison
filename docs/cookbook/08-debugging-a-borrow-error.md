# Recipe 08: Debugging a borrow error (B0010, B0060)

**Goal.** Read a `B0010` or `B0060` diagnostic, find the move site
or duplicate-borrow site, and apply the mechanical fix. By the end
you will recognise the three most common ownership mistakes and
turn each into a one-line edit.

**Prerequisites.** `ori` on `PATH`; familiarity with
[tutorial 04](../tutorial/04-effects.md). Background:
[`docs/language/MEMORY_MODEL.md`](../language/MEMORY_MODEL.md).

**Time:** ~15 minutes.

## 1. The borrow checker's two rules

The bootstrap borrow checker (`crates/ori-compiler/src/borrow.rs`)
enforces two invariants. **Aliasing XOR mutability**: a binding has
either one exclusive (`&mut`) borrow or any number of shared
borrows, never both — `B0010` and `B0011`. **Use-after-move**: a
value consumed (passed by value into a retaining closure, or
returned) cannot be referenced again — `B0060`. Two further codes
cover return-borrow issues: `B0050` and `B0080`. This recipe focuses
on `B0010` and `B0060`.

## 2. A working baseline

Save this as `src/bank.ori`:

```ori
module bank.ops

type AccountId wraps Str

type Account = {
  id: AccountId,
  balance: Int
}

fn debit(acct: Account, amount: Int) -> Account:
  return Account { id: acct.id, balance: 0 }

fn settle(acct: Account) -> Account:
  let after_debit = debit(acct, 100)
  return after_debit
```

Check:

```bash
ori check --json src/bank.ori; echo "exit=$?"
```

Empty stdout, `exit=0`. The baseline is fine: `settle` takes `acct`
by value, hands it to `debit`, and returns the result. No aliasing,
no double-use.

## 3. Triggering B0060 (use after move)

Modify `settle` so it tries to use `acct` after handing it to
`debit`:

```ori
module bank.ops

type AccountId wraps Str

type Account = {
  id: AccountId,
  balance: Int
}

fn debit(acct: Account, amount: Int) -> Account:
  return Account { id: acct.id, balance: 0 }

fn settle(acct: Account) -> Account:
  let after_debit = debit(acct, 100)
  return acct
```

The last line tries to return the original `acct`, but `debit`
consumed it. Run `ori check`:

```bash
ori check --json src/bank.ori
```

You will see a `B0060` diagnostic. The shape:

```json
{
  "schema":  "ori.diagnostic.v1",
  "id":      "B0060",
  "level":   "error",
  "message": "value `acct` is used in `settle` after being moved (consumed by closure or returned)",
  "agent": {
    "summary": "The binding `acct` was moved out by an earlier expression. Bind the result you want to use again, or borrow `acct` instead of moving it."
  }
}
```

Three things to read off the diagnostic. The binding name (`acct`)
is the value that was consumed. The function name (`settle`) is the
scope where the move and the later use both occur. The agent
summary names the two mechanical fixes: "bind the result" and
"borrow instead of move".

### Fix pattern 1: bind the result

`debit` returns a new `Account`. Bind it and use that — this is the
original baseline from §2. The mistake was treating `acct` as if it
still existed after `debit` consumed it; the fix uses the function's
return value.

### Fix pattern 2: pass by reference

If you need to read `acct` after `debit`, change the signature so
the call only reads. The canonical fix in the bootstrap subset is
to thread values through return types — split the consuming call
out of the path you need to keep readable:

```ori
module bank.ops

type AccountId wraps Str

type Account = {
  id: AccountId,
  balance: Int
}

fn read_balance(acct: Account) -> Int:
  return acct.balance

fn settle(acct: Account) -> Int:
  let bal = read_balance(acct)
  return bal
```

(Mutable-reference syntax (`&mut`) is prototype-only in the
bootstrap; the `&mut` pattern lands in milestone M23.)

## 4. Triggering B0010 (too many mut borrows)

`B0010` fires when a binding is borrowed mutably more than once at
the same call site. The production form lands with M23 region
inference; the diagnostic shape is:

```json
{
  "id":      "B0010",
  "level":   "error",
  "message": "parameter `acct` of `transfer` is borrowed mutably 2 times; at most one `&mut` is allowed per binding"
}
```

The fix is structural, not cosmetic. Three patterns:

### Fix pattern 1: split the binding

Make the two mutators operate on different bindings.

```ori
module bank.ops

type AccountId wraps Str

type Account = {
  id: AccountId,
  balance: Int
}

fn transfer(src: Account, dst: Account, amount: Int) -> Pair[Account, Account]:
  let new_src = Account { id: src.id, balance: 0 }
  let new_dst = Account { id: dst.id, balance: 0 }
  return Pair { first: new_src, second: new_dst }
```

Two distinct parameters: each carries its own borrow region; `src`
and `dst` cannot alias.

### Fix pattern 2: sequence the borrows

Split a nested expression into two consecutive statements. Each
statement opens a fresh region; a borrow that ends at the end of
statement one is gone before statement two starts.

### Fix pattern 3: convert to a return-value chain

Avoid in-place mutation entirely. Take ownership, return a new
value, let the caller decide what to do with it. Pure functional
flow has no borrow problems by construction.

## 5. Triage flowchart

When you see a `B****` diagnostic, walk this list:

1. Read the `message` field — the backticked name is the binding;
   the function name is the scope.
2. Open the source at the diagnostic's `span`.
3. `B0060`: find the earlier expression that took ownership.
   Apply pattern 1 (use the return value) or 2 (borrow instead).
4. `B0010` / `B0011`: find the second `&mut` (or the `&mut` next to
   a shared borrow). Apply pattern 1 (split) or 3 (return-value chain).
5. `B0020`: two newtypes share a base — check argument order.
6. `B0050` / `B0080`: a returned borrow has no source. Return by
   ownership, or borrow from a parameter that outlasts the call.

## 6. Using `ori agent diagnose` to find the move site

```bash
ori agent diagnose --json src/bank.ori \
  | jq '.top_repair_candidates'
```

Each candidate carries a confidence in `[0.0, 1.0]` and a Patch IR
document. For `B0060`, the bootstrap typically returns a
`replace_node` patch using fix pattern 1. An agent harness should
auto-apply only at >= 0.85 and surface the rest to a human.

## 7. The deeper lesson

The borrow checker is pessimistic by design — every `B****`
diagnostic is correct under the bootstrap's analysis, with no
false-positive class. The M22/M23 region-inference work will narrow
the cases that need manual rewrites; until then, the three patterns
above are the canonical toolbox.

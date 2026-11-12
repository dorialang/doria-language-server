# Semantic hover

## User contract

`doria-lsp` provides the same hover behavior to VS Code and JetBrains clients through
the standard `textDocument/hover` request.

For a compiler-resolved declaration or reference, hover returns:

- the canonical Doria declaration signature;
- the declaration's immediately preceding PHPDoc comment, when present;
- the exact source range of the hovered identifier.

The first vertical slice covers same-file classes, free functions, instance methods,
and static methods. Method references are resolved from compiler-produced expression
types, including nullable receivers and `self`/`parent` static qualifiers. Hovering a
declaration uses the same symbol record as hovering one of its references.

Enum declarations use the same compiler-owned snapshot. Unit and backed cases show
their nominal enum result type, payload cases show their declared field signature,
payload fields show their resolved type, and the readonly `value` projection shows
the backing type. Enum and payload-case documentation identifies whether the enum is
a Copy or Move value without exposing private layout offsets. Static completion after
`EnumType::` lists only cases declared by that resolved enum. The server does not infer
cases from PascalCase spelling.

If a document has semantic errors, declarations that still parse remain available.
References are returned only when the compiler can resolve them safely. Existing
keyword, primitive type, and compiler-intrinsic hover remains available as a lexical
fallback.

Decision 0139 local `where (...)` clauses are retained in authored conformance,
method, computed-property and accessor signatures. Only compiler AST clause spans
receive contextual keyword hovers and semantic highlighting; ordinary `where`
function, member and variable names remain identifiers. TextMate recognizes
completed lexical clause heads; AST semantic tokens also cover heads split
across lines. The compiler checks guarded bodies with scoped capability premises
and checks member availability against the caller's bounds after inference.
The server preserves E0779 language diagnostics at call/access spans with related
locations at the authored condition, as well as E0777 placement errors.
Accepted conditional declarations no longer produce the temporary E0776
execution rejection. Checked compiler member surfaces, call signatures, targets
and accessor facts drive editor features; conditional members stay excluded from
unconditional authored completion and callable-resolution fallbacks. Occurrence
availability facts retain the receiver span and caller premises for instance and
static completion. Unproved members are excluded; candidates requiring method
type arguments retain their condition and remain subject to invocation checking.
Declaration inventories remain intact for identities and rename. Compiler
global-name and alias facts remain available. The shared
`stage36a-conditional-capabilities.doria` fixture and server regressions pass
against the local compiler on macOS ARM64. Editor builds, pin integration and
installed-tool refresh remain held. No LSP metadata schema fields are added.

Resolved declaration predicates in compiler-owned `ContractFacts.conditions`
provide lexical subject identities for same-source definition navigation and
type-parameter hovers. Same-spelled parameters in different owners stay distinct;
invalid subjects retain E0778 at their exact token and publish no predicate.
These facts do not activate members or conformance witnesses. Full generic-use
references and rename remain unavailable because clause-subject facts do not
enumerate parameter uses in signatures, bodies or interface arguments. Navigation
and diagnostic regressions pass in the local-compiler server verification.
Guarded-body protocol regressions cover scoped premises, caller-bound isolation
in both declaration orders, concrete and null-safe calls, named arguments,
result inference, one shared condition for computed read-modify-write, and core
display/equality/ordering bounds. Unresolved inference retains E0531; unproved
ordinary symbolic generic-call bounds receive E0535 without a later concrete
instantiation. Checked conditional programs publish semantic facts for admitted
members and conformances. Regression coverage requires unavailable members to be
absent from the selected receiver surface and retains compiler-known unconditional
conformances. Availability selection belongs to the compiler's member-surface
contract. These protocol and editor-feature cases pass in the local-compiler
server verification; Linux, Windows and installed-editor behavior remain untested.

The lexical fallback describes the accepted Stage 30a callable grammar. Stage 30c
semantic hovers use compiler metadata instead: structural function-typed bindings
and parameters show their canonical semantic type and callback ownership contract;
closure expressions show inferred invocation mode and required checked effects together with
owned or borrow-bound provenance, capture acquisition, invocation consumption, and
escape classification. Capture occurrences show their compiler-resolved acquisition
mode. Callable-value calls show the checked function signature where the existing
expression-hover route can identify the call. Authored effect order remains visible
on source function types. A function value narrowed from `mixed` or a nullable
binding shows the compiler-resolved structural function type at that use, while
the declaration retains its authored `mixed` or nullable type. Arity, parameter
types and ownership, invocation mode, return type, required checked effects, and nullability
remain distinct; the server does not substitute a generic callable label. The
server does not rediscover free variables, infer
captures, or calculate lifetimes from text.

Compiler-owned ambient I/O effects are supplementary runtime facts rather than
source structural-function identity. Closure, callable, and property-access hovers show
required source effects in the signature and list exact ambient I/O effects in a
separate **Ambient I/O** section. The server does not derive that classification
from an error's source spelling. Abstract getter/setter dispatch retains the
compiler's automatic transport profile; concrete effect-free getters do not
acquire ambient effects merely because the abstract contract can carry them.
Hooked-property completion details use the same accessor signature for concrete,
erased-interface, and constrained-generic receivers. The getter is shown when
present, otherwise the setter; documentation retains both accessor contracts.
Class modifiers remain visible, and ordinary stored fields keep field signatures.

The compiler publishes precise capture and callable diagnostics with safe capture
fixes. Ordinary language-server analysis is target-neutral, so valid closures and
type-only function syntax do not receive `E0641`; invalid closures receive their
specific semantic diagnostic without a redundant execution boundary. `E0641`
is historical and reserved; the server does not filter or suppress it. Stage 30
is complete. Semantic closure hovers
identify debug and native execution for a valid closure. They describe explicit
PHP closure lowering separately and condition it on the program's value families
and operations being supported by the PHP backend; target-neutral analysis does
not claim per-closure PHP compatibility. On a resolved `List<T>`, completion
offers `map`, `filter`, and `reduce`, and call hover renders the compiler's
concrete specialized callback and result types, selected readonly or writable
repeatable access, required checked effects, ambient I/O effects, unchanged-source
contract, and owned result.
The server consumes `ListAlgorithmCallInfo`; it does not infer callback effects
or reconstruct an algorithm type checker. Other collection families do not
receive these algorithms. PHP remains a secondary compatibility backend with
independent limitations. Stages 31 through 34 are complete. Stage 35 interfaces
and trait declarations, graphs, and conformance are implemented in Slice 1;
Slice 2 implements interface values, erased calls, and shared interface payloads. Requirement hovers
show authored receiver, parameter ownership, generics, and checked effects
without presenting the requirement as an executable body. Semantically invalid closures
receive no execution capability block.

Interface member completion reads the compiler's specialized requirement graph,
including inherited requirements and Error messages. It never selects arbitrary
members of a known implementer. Erased-call signature help preserves requirement
parameter names, ownership modes, generic substitutions, and checked effects;
implementation-only defaults do not appear. Shared owner/access completions
preserve wrapper precedence and readonly/writable forwarding. Core operations
and public iteration are implemented. Composed member hovers use the selected
specialized signature, ownership, defaults, checked effects, and return-borrow
facts. Hovering shared trait source preserves the separate composer contexts,
including dependency sources shared by multiple workspace package graphs.
Signature help lists each distinct checked specialization, and completion retains
the corresponding signatures rather than choosing the first composer;
aliases and adaptation candidates retain their authored implementation origins.
`borrow` constructor promotion is presented as a retained readonly source loan,
not an owned field. Keyword completion and hover explain the same source-loan
boundary. Compiler return-borrow facts describe `getCurrent()` for
Move elements; exact-dynamic-result facts describe owned `clone()` results.
Preserving collection operations and `List::filter` accept Copy-or-Cloneable
elements without introducing implicit cloning in ordinary assignments.
Nullable hash-key and ordered-element rejections are forwarded from the compiler;
the server does not infer payload conformance for nullable types.

Property-hook hovers preserve compiler-resolved setter types and ownership modes;
the setter input type is exactly the property's type. Automatic backing in an
override reuses the inherited field. Backing initializers run parent then child
without invoking setters; a child initializer replaces the inherited value.
Hooks cannot perform blocking I/O, including output or blocking helper calls;
diagnostics and transitive effect checks belong
to the compiler. An ordinary interface getter may return a new owned callback
that borrows its receiver, but the callback cannot outlive that receiver.
`borrowed get` instead lends an existing callback. Hover keeps ownership of the
returned value separate from loans retained by its captures, using compiler facts
rather than inferring lifetimes from source or return-type spelling.
Compiler lifetime metadata also distinguishes source loans that end at the
result's last use from loans required through cleanup, even after the last use.
Callable and retained-source hovers preserve that distinction.
For typed ownership-taking callbacks, declared owned-result retention is a
permitted upper bound, not a requirement to invent a loan on every result.
The same `function(take Cursor): ?Cursor` annotation accepts forwarding the
consumed cursor or consuming it and returning loan-free `null`. Likewise,
`function(take List<Cursor>): List<Cursor>` may forward its consumed list or
return a fresh empty list. Actual source loans, exclusivity, and cleanup remain
protected; ordinary returned borrowed aliases are unchanged. The shared
owned-source-carrier fixture covers both loan-free results with source mutation
before later result use, plus a negative nullable-forwarding mutation control.
These source regressions await compiler-matched tooling validation.
[Decision 0136](../../doria/docs/decisions/0136-independent-owned-property-borrowing.md)
in the sibling compiler repository distinguishes canonical stored property
paths, including inherited generic fields and nested paths: disjoint owned
sibling fields may be borrowed independently. Readonly parent or field access
still cannot grant writable permission. A live field loan protects that field and every
ancestor against movement, replacement, or destruction; aliases of the same
field still conflict. Returned borrowing helpers preserve those paths; direct
owned-property move-out remains rejected. This does not establish disjoint
collection indices or byte ranges, nor exemptions for computed properties or
retained borrowed fields.
The compiler owns path identity and conflict checking. IDE regressions for this
rule are prepared pending compiler implementation and validation; clients do not
implement their own field-sensitive checker or suppress ownership diagnostics.

Stage 34 class and method hover consumes compiler-owned hierarchy metadata.
Class hover shows the canonical declaration, direct parent, hierarchy depth,
open/closed role, and runtime representation without exposing descriptors or
layout details. Method hover preserves its complete source contract and adds the
virtual root, nearest override, inherited defaults, and direct-versus-virtual
facts labeled **Source Dispatch**. These describe language-level resolution,
not the optimized machine-code shape; devirtualization and inlining do not change
source definition or rename targets. A `parent::` call names its exact direct
parent declaration and states that virtual dispatch is bypassed. The server does
not reconstruct a hierarchy or override checker from source text.

Constructor parameter hover likewise consumes compiler-owned role facts. A
promoted parameter identifies its declared property, external or internal
accessibility, and property mutability. A `parameter` input is identified as
constructor-only and as declaring no property. A constructor `override` input
names the inherited root property, states that it creates no storage, and keeps
the parameter's borrow or ownership mode separate from the root property's
mutability. Role-preserving signature help and named-argument completion use the
same callable symbol. No hover exposes physical slots, backend symbols, or
runtime descriptors.

Foreach binding hover is driven by compiler-owned foreach semantic facts. A
`List<T>` or `T[]` first binding is shown as a readonly **Zero-Based Sequence
Index** with canonical type `int`; Dictionary and SortedDictionary first
bindings are shown as readonly **Dictionary Key** values with their authored key
type. Element/value hover independently preserves its resolved type and readonly
or writable access. Decision 0133 requires those types to remain visible in
source on every binding. The server does not infer these roles from source
spelling or short type names; compiler inference on an omitted type is recovery
for diagnostics and fixes only. Interpolation and `%s` remain materialized strings and do not
create a primitive `toString` or scalar-cast hover/completion surface.

Attribute application hover uses compiler-owned Stage 32 metadata. It shows the
canonical attribute class, declaration target, constructor parameter types,
evaluated constant values, and whether a value came from a default without
exposing compiler IDs or host paths. Attribute-schema hover supplements the
ordinary rich class hover with its constructor-shaped metadata contract.
Compiler-known `Attribute`, `Test`, and `PHPExport` hovers state their metadata
role and their deferred execution or bridge boundary. The server does not run
attribute constructors, provide runtime reflection, or infer attribute facts
from source spelling.

Ownership hover text stays in Doria vocabulary: Owned Closure or Borrow-Bound Closure,
Readonly/Writable/Owned taking capture, Readonly/Writable Repeatable or Consumes On
Invocation, Nonescaping or Owned callback, and returned closures tied to a parameter
or `$this`. Nonescaping describes the source lifetime contract, not guaranteed
physical stack placement. Compiler-private binding IDs, closure coordinates,
pass slots, future environment layout, and backend symbols are never presented.
Borrow-bound provenance is not a blanket collection-storage rejection: the
server preserves compiler acceptance of lifetime-safe literals and its findings
for live-source conflicts or escapes to longer-lived storage. Nonescaping
callback parameters remain non-retainable.

Constructor-rooted writable paths and owned property writes use the same compiler
authority. Accepted nested writes, owned initialization, and writable replacement
remain diagnostic-free; readonly intermediates, uninitialized intermediates,
borrowed right-hand sides, and overlapping transfers retain the compiler's exact
codes, labels, severity, kind, and metadata. The server does not infer path
capability or constructor state, and it does not claim support for moving values
out of properties.

Compiler-known ownership and collection methods use the same signature-first
presentation. When the compiler resolves the receiver, hover substitutes its concrete
generic arguments and exact return type; for example, `WeakReference<Theme>::acquire()`
returns `?SharedReference<Theme>`. In incomplete code, the lexical fallback still shows
the generic callable signature rather than documentation alone.

Compiler-known built-ins use signatures and checked-error effects exported by `doriac`.
Compiler-known I/O types are completed and hovered only by their canonical qualified
identities under `Doria\Std\Io`; short aliases such as `IoError` are neither guessed nor
offered. Because these declarations are compiler-known rather than source-backed, the
server provides their contract documentation without inventing a definition location.

`Bytes` and the inline `ByteRegion` / `WritableByteRegion` contracts come from
compiler-owned typed byte metadata. Member completion, hover and signature help
share its parameter names, result types and receiver access requirements. Region
documentation distinguishes borrowed source provenance from owned storage; a
region is not presented as a synthetic owning class.

The `Doria\Std\Io` stream declarations use ordinary compiler nominal, member,
and requirement facts. Completion and hover retain the exact
outcome payloads, writable receiver and destination modes, result types, and
checked effects; `DuplexStream` inherits reading and writing, not flushing or
closing. Qualified fallback documentation displays the compiler's parsed
declaration text instead of maintaining another signature inventory. Imported
aliases require normal name resolution, and compiler-known contracts have no
invented source definition location. `File::openRead` returns an owned
`ReadableFile`; `openWrite` and `openReadWrite` require explicit `path`,
`FileCreation`, and `FileWriteMode` arguments with no defaults and return owned
`WritableFile` and `DuplexFile` values. `FileWriteMode` declares `Positioned` and
`Append`. File capabilities expose writable `read` and/or `write` and consuming
`close`, with compiler-owned ambient `IoError` effects. `ReadableFile` explicitly
implements `PendingReadableStream`, `WritableFile` implements
`PendingWritableStream`, and `DuplexFile` implements `DuplexStream` plus both
pending capabilities. Metadata-only class hovers list checked interface
conformances from compiler semantic facts; member help follows the same compiler
inventory, including the pending preparation/completion signatures. These
declarations do not imply delivered public group execution. Their constructors are
internal, and reuse after close follows compiler move diagnostics. Synchronous
file operations execute in the interpreter and native backends. Generated native-backed
PHP uses the compiler-published matching runtime through FFI; missing, disabled,
or mismatched dependencies report `B3602`. Pure PHP stream adapters do not need
FFI, and target-neutral IDE analysis does not infer deployment availability.

Andrew approved `Doria\Std\Io\SeekableStream` with writable
`seek(int $offset, SeekOrigin $origin): int throws IoError`. The required
`Doria\Std\Io\SeekOrigin` enum declares `Start`, `Current`, and `End`; seek
returns the new position. `ReadableFile`, `WritableFile`, and `DuplexFile`
implement this capability and expose readonly `getPosition(): int throws IoError`
and `getLength(): int throws IoError` methods. Andrew revised the earlier
proposal: the `position` and `length` property APIs and examples are removed.
Fresh inspection may block, so these methods remain forbidden in property hooks,
including through helper calls. Decision 0135's global nonblocking hook rule is
unchanged; `E0770` is not suppressed. Seek failures append `IoOperation::Seek` after
`Access` without changing earlier tags. Inspection failures use `Access`.
Both preserve the opening-path `IoTarget::File`, null `transferredByteCount`,
and an optional OS `systemCode`. Seeking does not change append mode: each append
write still uses the then-current EOF.
The working compiler registers the seek declarations. The compiler task reports
macOS execution and direct/transitive hook checks passing; Linux/Windows runtime
checks were compile-only. These are not tooling validation or Stage 36a delivery.
Shared file
help describes the approved contract but does not synthesize members, cases,
or source locations. The prepared `stage36a-seekable-files.doria` fixture and
source regressions cover all three file families, an imported interface/origin
alias, readonly checked inspection methods, writable seek, named arguments, all origins,
and rejected old property reads, readonly seek, and missing origin. A prepared
diagnostic-parity regression rejects direct and helper-mediated inspection calls
from hooks for every file family. Existing
`IoOperation` exhaustiveness coverage includes `Seek`; the original compiler
diagnostics remain unsuppressed. Tooling builds and behavioral validation,
compiler-pin integration, and installed-tool refresh remain held.

Andrew subsequently approved writable `synchronizeData(): void throws IoError`
and `synchronizeMetadata(): void throws IoError` on `WritableFile` and
`DuplexFile` only. Metadata synchronization includes full data plus metadata;
`ReadableFile` exposes neither method. No new interface or abbreviated aliases
are introduced. Failures append `IoOperation::SynchronizeData` and
`SynchronizeMetadata` after `Seek`, preserving earlier identities, and use the
original opening-path `IoTarget::File`, optional `systemCode`, and null
`transferredByteCount`. Shared help is limited to the writable file families.
The file fixture includes both methods on both eligible owners; prepared
regressions preserve writable-only completion, readonly/readable rejection,
checked effects, and distinct operation exhaustiveness. The compiler task reports
focused macOS compiler/runtime suites and strict checks passing; Linux/Windows
runtime checks were compile-only. The recorded combined synchronization/resize
native matrix now supersedes the intentionally stopped synchronization-only run.
Tooling builds and behavioral validation remain held.

Andrew approved writable `setLength(int $length): void throws IoError` on
`WritableFile` and `DuplexFile`, not `ReadableFile`. Its required byte length is
nonnegative. Shrinking discards the tail; supported growth exposes zero-readable
extension. The cursor is unchanged and the owner is not consumed. Checked
filesystem/platform failures append `IoOperation::Resize` after
`SynchronizeMetadata`, preserving earlier tags, with the original opening-path
`IoTarget::File`, optional `systemCode`, and null `transferredByteCount`.
No truncate alias or interface expansion is introduced. The shared file fixture
and broadened writable-file regression cover both eligible owners, the required
named `length` argument, readable/readonly rejection, wrong or missing arguments,
and the appended operation. Runtime shrink/growth/cursor guarantees remain the
compiler/runtime suites' responsibility. The compiler's `current-pipeline.md`
records the combined macOS checkpoint, including the full interpreter/Cranelift/LLVM
matrix and focused PHP execution. Linux/Windows checks were compilation only,
not execution. Tooling tests remain unrun, and compiler-pin integration,
installed-tool refresh, and delivery are not established by that checkpoint.

`ReadinessStream` exposes `ReadinessSource $readinessSource { borrowed get; }`.
Readable, writable, and duplex file owners implement this capability separately
from their transfer interfaces. The opaque `ReadinessSource` has no public
constructor or operations. Getter formation performs no I/O or allocation;
custom adapters forward the receiver's normal source loan rather than an owned
handle. Hover and completion preserve that readonly borrow and its last-use
lifetime using compiler property-accessor facts. Waiting uses the shared native
core while transfers dispatch through the complete original adapter. The
broader readiness-set and transfer-group surface remains in progress; these
declarations do not imply that it has been delivered. The compiler now declares
`Doria\Std\Io\StandardStreams` with readonly static `input`, `output`, and
`error` properties returning `StandardInput`, `StandardOutput`, and
`StandardError`. Decisions 0110 and 0138 define these views as non-owning,
nonclosable inline Copy values over the process's existing devices. Access and
copying allocate no native handle, control block, buffer, or heap object. Copies
share intrinsic redirection and access/mode coordination. Exclusive mode changes
still require a restoring guard. Tooling uses the compiler's `InlineValueKind`
and nominal conformance facts for type, binding, and imported-alias help.

Copy-to-interface erasure creates an independent owning snapshot. A returned
borrow stored beyond its call statement requires a named interface owner; the
original Copy binding remains independently assignable. Nullable erasure obeys
the same lifetime rules. The shared `stage36a-standard-streams.doria` fixture
covers accessors, generic Copy transport, nullable views, explicit erased owners,
byte transfers and outcomes, registration keys, and consuming transfer plans.
`FlushableStream.flush()` returns the nominal Copy enum `FlushOutcome`, with
payload-free `Complete` and `Pending` cases. Pending retains the unwritten
remainder for a later call; flushing does not implicitly wait for readiness.
Checked failures still throw `IoError`. Hover, completion and signature help use
the compiler-owned return type. A void-returning implementation no longer
satisfies the interface contract.
Prepared diagnostic regressions preserve `E0478` for escaping implicit erased
temporaries. Compiler-owned metadata supplies readonly accessor help, member
directions, checked effects, and source identity. Local-compiler library and
protocol tests pass on macOS ARM64. Editor builds, pin alignment and installed-tool
refresh remain held.
Remaining lock, adapter, mode-control, and process spellings stay deferred to
their compiler-owned decisions.

Decision 0110's **Completion Observation** and **Completion Wait Budgets**
contracts distinguish completion-fetch wait expiry from transfer deadlines and
group lifetime control. An immediate or finite wait that expires produces a
typed timeout observation, not a completion, an exhausted group, a nullable
transfer outcome, or a fabricated `IoError`. Expiry stops only the wait;
transfers remain active, retaining group storage loans, identities, and actual
progress. Callers may wait again and reuse completed storage through the group,
including resubmission. Explicit cancellation, group deadlines, and teardown
still cancel and drain outstanding transfers before storage release. Andrew
approved optional nullable `deadline` and `cancellation` parameters on `begin`
using the existing `Deadline` and `CancellationToken` types, both defaulting to
null. Null omits that lifetime control. Controls persist for the group, which
retains shared cancellation-token state rather than a `CancellationSource`
loan. No `TransferControl` type is introduced. Untimed waits require no clock
sampling or per-wait allocation. Fetching a failed transfer's completion raises
its original checked error exactly once, with the failed operation's identity
available before propagation; it does not cancel other transfers or release
group loans. Public signatures and timeout diagnostic descriptions must follow
the compiler's finalized declarations, not an editor-specific inventory. These
approved contracts do not claim delivered public group execution.

The approved `Doria\Std\Io\RegistrationKey<T>` contract is a type-preserving,
non-owning inline Copy key: copying a key leaves both values usable. The set owns
the complete registered stream, not the key. Copied keys may become stale;
stale or foreign-set uses must be rejected. A key grants no independent stream
borrow or public platform handle. Compiler declarations and generic Copy facts
remain the authority for tooling recognition.

The compiler declares `ReadinessSet::__construct()` and its writable generic
`register<T implements ReadinessStream>(take T $stream): RegistrationKey<T>`
and `unregister<T>(RegistrationKey<T> $key): T` methods, each with checked
`IoError`. Registration requires nominal readiness, not transfer or pending
capabilities; removal preserves the complete original type. Shared metadata
supplies hover, signature help, named arguments, and writable-only completion.
Class-call presentations keep compiler-classified automatic I/O in effect
documentation, distinct from source-required `throws` in signatures.
The readiness-set fixture covers aliases, qualified names, concrete, erased,
generic, and readiness-only adapters. Construction lowers through the compiler's
checked-I/O path; its separate fixture covers owning properties, collections,
nullable values, and checked exits without accepting any diagnostic. Registration
and removal still reach the compiler's structured unsupported-development
boundary until their runtime lowering lands; membership tests allow that boundary
or successful analysis, never false syntax, symbol, type, or ownership errors.
Tooling forwards the boundary without suppression.

Andrew approved readiness-set borrowing names `getStream` and
`getWritableStream` on 2026-10-10. Help derives the registered return type,
readonly/writable access, and receiver-rooted loan from compiler metadata;
the key does not become an independent borrower. The shared borrowing fixture
covers imported aliases, qualified names, concrete, erased, and generic keys.
Signature help and named arguments share those compiler facts, and readonly
set completion does not offer writable access. Source regressions are prepared;
current-source compiler/LSP execution remains pending.

Tooling uses the compiler's shared `InlineValueKind` classification, not the
backend's specialized storage identity, for Copy-type hover and completion.
Bindings preserve the complete resolved generic type, including nullable forms
and distinct readable/writable stream arguments. The shared registration-key
fixture covers imported aliases, qualified names, Copy reuse, and nullable
constant defaults. Negative cases retain the compiler's diagnostics for wrong
type arguments, integer conversions, opaque construction, and missing arguments.

Andrew approved inline Copy `CancellationToken` on 2026-10-11 and its canonical
namespace on 2026-10-12: `Doria\Std\Concurrency\CancellationToken`. Assignment
leaves both bindings usable with the same explicitly shared cancellation state.
Each independently owned copy retains one state claim and releases it during
ordinary cleanup. Borrowed arguments acquire no ownership claim. Copying
allocates nothing; token creation and copying allocate no second state or native
signal. Tokens may outlive their source, and copying or dropping a
source/token does not request cancellation. This is a type-specific sharing
contract, not a change to ordinary Doria ownership.

Andrew subsequently approved shared-state identity equality: `==` compares the
shared cancellation state, and `!=` is its inverse. Tokens and copies from one
source compare equal; distinct live states differ. Cancellation and source
destruction do not change identity while tokens survive. Ordinary nullable
equality applies. Borrowed comparisons acquire no claim and perform no signal
or clock work. This grants no ordering, hashing, integer conversion, or address
API. Shared token help and the cancellation fixture cover these comparisons;
tooling behavioral validation remains held.

Copy classification alone does not imply absence of retained ownership. The
existing time/key help's non-owning description is limited to those types and
must not be reused for the cancellation token. Compiler-owned type and cleanup
facts supply token recognition once its declarations exist. The shared Copy
description makes no blanket claim of trivial cleanup. Non-owning inline help
is selected using the compiler's cleanup classification, while the compiler's
`InlineValueKind::CancellationToken` selects shared-state ownership help. A
source regression covers all inline kinds without synthesizing public token
declarations; behavioral validation and pinned-compiler integration remain held. Andrew
subsequently approved `Doria\Std\Concurrency\CancellationSource` as a Move
source constructed with `new CancellationSource()`, with readonly
`token: CancellationToken` and `cancellationRequested: bool` properties and
`writable cancel(): void throws CancellationError`.
`Doria\Std\Concurrency\CancellationError` implements `Error` and exposes
readonly `message: string` and `systemCode: ?int`. Cancellation remains requested
if notification fails. No `create()`, `getToken()`, or `requestCancellation()`
member is introduced. Tooling adds no synthetic declaration, Copy classification,
or client-side member rules. The working compiler now supplies the Concurrency
declarations. Type completion and fallback hover select Concurrency's own
declaration text and source identity rather than Stream declarations. Semantic
member help preserves readonly getters and the writable checked `cancel()`
signature, including imported aliases, without treating compiler declarations
as authored IDE locations. Token help remains opaque and exposes no constructor
or token status property. These source updates await tooling validation,
compiler-pin integration, and installed-tool refresh; they do not establish
Stage 36a delivery.
The shared `stage36a-cancellation.doria` fixture and
`cancellation_source_and_copy_tokens_have_no_false_diagnostics` regression
are prepared for compiler integration, not behaviorally validated. They cover
the approved constructor, properties, checked error, Copy reuse, and a token
returned after its source leaves scope. Runtime notification-failure and
retain/release verification, including equality stability, belong to the
compiler/runtime suites. The fixture also covers same-source tokens and copies,
distinct sources, comparisons after cancellation and source destruction,
nullable comparisons, and reuse after borrowed comparison. A focused hover
regression checks that comparison operands retain shared-state identity help
without the non-owning inline claim.
A source regression preserves Move diagnostics for ordinary user-defined
`App\CancellationToken` and `App\Concurrency\CancellationToken`. Neither a
matching short name nor a `Concurrency` namespace grants standard-token semantics. Token
execution and retain/release behavior remain unvalidated in tooling.

Andrew approved distinct inline Copy `ReadTransferKey` and `WriteTransferKey`
identifiers on 2026-10-11. Their help describes transfer identity, not stream
registration or resource ownership; neither substitutes for `RegistrationKey<T>`
or integers. The compiler inventory supplies their nominal classification.
The exact plan pairs are `read` / consuming `andReadFrom` and `writeTo` /
consuming `andWriteTo`; the earlier `andRead` spelling is removed, with no
legacy aliases. Compiler signatures supply pending capability bounds, region
access, and checked setup effects. Read plans retain exclusive writable
destinations; write plans retain shared readonly sources, including mixed
appends. Configuration performs no transfer. Focused source regressions cover
these contracts; execution remains pending. This adds no unsettled group APIs.

Andrew subsequently approved readonly `getReadKey(int $position)` and
`getWriteKey(int $position)` retrieval on `TransferPlan`. Positions are zero-based
across the entire mixed plan, not separately counted by direction. Results are
the corresponding nonnullable typed keys; negative, out-of-range, or
wrong-direction positions throw existing `IoError` with
`IoOperation::Access`, `IoTarget::TransferPlan`, and `IoErrorReason::InvalidInput`;
The `TransferPlan` target is payload-free; `IoError` has no public `path`
property. `systemCode` and `transferredByteCount` are null. Key copies retain stable
operation identity through consuming appends without retaining source/buffer
loans. Source tests cover qualified names, aliases, readonly completion, named
arguments, exact signatures, and Copy results surviving plan consumption.
Tooling test execution and pinned-compiler integration remain pending under the
existing holds. Compiler declarations now contain both readonly getters.

The approved completion-observation contract uses
`getNextCompletion(Wait)` with one inline Copy `Doria\Std\Io\Wait` enum:
`Immediate`, `For(Duration $duration)`, `Until(Deadline $deadline)`, and
`Indefinite`. Its approved result is the ordinary inline Copy enum
`TransferWaitResult`: `Completed(TransferCompletion $completion)`, `TimedOut`,
or `Exhausted`. `TransferCompletion` is a separate ordinary inline Copy enum:
`Read(ReadTransferKey $key, ?ReadOutcome $outcome)` or
`Write(WriteTransferKey $key, ?WriteOutcome $outcome)`. Their canonical compiler
declarations supply hover signatures, case completion, imports, payload types,
and Copy classification; no short-name prelude aliases or editor enum engine
are added. Source regressions cover qualified names, import aliases, nested
construction, named payload arguments, nullable outcomes, and reuse after Copy.
Tooling execution remains held at the old compiler pin.

Nullable completion outcomes use ordinary same-nominal-enum equality under
Decision 0114, not an I/O-specific exception. Source regressions compare
pattern-extracted nullable outcomes with both `==` and `!=`, and preserve
compiler diagnostics for incompatible nominal operands.

`Completed(Read($key, null))` is an actual retired completion with no transfer
outcome, not absence of a completion. Zero-byte success, `WouldBlock`, EOF, and
read `TimedOut` remain actual outcomes in their applicable completions. Failed
transfers still throw the original owned checked `Error`, not an enum error
case. If wait infrastructure fails alongside a preserved transfer completion or
owned transfer error, the infrastructure error is reported first. The original
completion or owned error remains available for a subsequent fetch; it is not
discarded, relabeled, or reconstructed. This is the approved ordering contract,
not a claim of validated tooling or public group execution.
Wait timeout and exhaustion do not permanently prevent resubmission.
Constructing or copying these values does not retire work, release retained
loans, or grant buffer access. The approved reusable owner is
`Doria\Std\Io\TransferGroup`, with readonly
`observedReadKey: ?ReadTransferKey` and `observedWriteKey: ?WriteTransferKey`.
Only the fetched operation's direction is present. Both properties are null
before the first fetch and after timeout or exhaustion. They publish the
consumed plan's original opaque stamp and mixed-plan operation position before
the original owned checked `Error` propagates, preserving that identity across
resubmission. Copies retain no group, storage, or source loans or resources.
Observed keys do not replace the typed keys in successful completion payloads.
Neither `TransferIdentity` nor `TransferSession` is a public type.

Shared key and wait-result descriptions restate this approved contract, with
source assertions for the exact names and nullable readonly properties.
The compiler symbol inventory declares `TransferGroup`. The approved writable
`begin` contract is:

```doria
ReadinessSet::begin(take TransferPlan $plan,
    ?Doria\Std\Time\Deadline $deadline = null,
    ?Doria\Std\Concurrency\CancellationToken $cancellation = null): TransferGroup throws IoError
```

The working compiler declares both null defaults. The compiler task reports
control execution through typed MIR, interpreter, Cranelift, LLVM, and PHP/FFI;
that is not tooling validation or a completed Stage 36a claim. Tooling has not
run behavioral validation and does not synthesize the signature. The shared
`stage36a-transfer-group-controls.doria` fixture covers named controls, omitted
and explicit-null controls, a token surviving its source, and the separate
`Wait::Immediate` completion budget. A prepared source regression requires
compiler-owned signature, named-argument, hover, and default-availability facts.
Readonly nullable `observedReadKey` and
`observedWriteKey` getters are now declared too. Source regressions cover their
canonical and aliased types, detached Copy results, and readonly diagnostics.
`getReadRegion` is now declared with a readonly retained receiver loan whose
lifetime is `LastUse`. Writable resubmission and
`getNextCompletion(Wait $wait): TransferWaitResult throws IoError` are now
declared as well. Tooling uses those compiler-owned declarations, not substitute
signatures. Source assertions cover fetch hover, completion, named arguments,
readonly filtering, and rejection of fetching while a read-region loan is live.
They also cover chained `&&` and `||` conditions where a completed operand's
call loan ends before a later operand mutates the group. A negative outer-call
argument case preserves the earlier argument's region loan while evaluating a
later short-circuit argument; that enclosing loan must not be released early.
Tooling behavioral validation and pinned-compiler integration remain pending.

Andrew subsequently approved readonly
`TransferGroup::getReadRegion(ReadTransferKey $key): ?ByteRegion` on
2026-10-11. For a valid key it returns null while the operation is active or
has no known completed byte range. A known zero-byte completion, including EOF,
returns a present empty readonly region; other known progress exposes exactly
the completed prefix of the original buffer. No active buffer is exposed.
Retirement is distinct from fetching: an already-retired Immediate request's
known completed prefix is accessible before fetching its completion, while
observed keys remain null until fetching. Fetching and group cleanup must not
repeat the retired adapter callback. An original queued `IoError` remains owned
until it is fetched or the group is destroyed; no error subclass is required.
Access performs no I/O, allocation, or observation mutation. Its readonly borrow
preserves group/storage loans and must end before resubmission or group drop;
resubmission makes the previous prefix unavailable.

Invalid, foreign, or wrong-direction keys throw existing `IoError` with
`IoOperation::Access`, `IoErrorReason::InvalidInput`, and the newly appended,
payload-free `IoTarget::TransferGroup`, not `IoTarget::TransferPlan`.
`systemCode` and `transferredByteCount` are null, and there is no public `path`
property. The additional target requires an arm in exhaustive `IoTarget`
matches. Source regressions require compiler-owned method signatures, nullable
readonly-region bindings, named arguments, and the new target declaration;
they remain unrun. The compiler now declares both the payload-free group target
and completed-region member. The returned descriptor retains the readonly group
loan through its last use, not through cleanup after last use. Source assertions
cover that metadata, mutation after last use, rejected conflicting mutation,
and owner escape without suppressing compiler diagnostics. The compiler task
reports public fetch, completed-prefix access, and original-key resubmission
passing interpreter, Cranelift, and LLVM execution. Tooling has not independently
run those tests. Description tests do not establish runtime null/empty/prefix
behavior. The compiler task now reports all 34 transfer-group regressions
passing on macOS interpreter, Cranelift, and LLVM; that report does not establish
tooling validation, Linux/Windows execution, or Stage 36a delivery.

The approved reuse names are `resubmitRead` and `resubmitWrite`; `begin` starts
the initial transfers. Each reuse selects the original directional operation
key and a checked range of its retained buffer after the previous operation
retires and conflicting region loans end. It creates no new operation or key,
and makes the previous completed prefix unavailable. Read and write keys remain
distinct. No `submitRead` / `submitWrite` aliases are added. Invalid keys or
ranges throw existing `IoError` with `IoOperation::Configure`,
`IoTarget::TransferGroup`, and `IoErrorReason::InvalidInput`. The target is
payload-free; there is no public path, and `systemCode` and
`transferredByteCount` are null. Failed validation submits no work and consumes
no stored transfer `Error`. Actual transfer failures propagate unchanged through
`getNextCompletion`. Shared key descriptions distinguish this configuration
failure from completed-region `Access` failures and plan-position lookup.
The compiler now supplies the exact checked signatures:
`writable resubmitRead(ReadTransferKey $key, int $offset, int $length): void
throws IoError` and `writable resubmitWrite(WriteTransferKey $key, int $offset,
int $length): void throws IoError`. Unobserved or active operations cannot
discard results or submit more work. The original key and retained buffer
survive reuse. Source assertions cover signatures, named ranges, directional
Copy keys, readonly filtering, and last-use conflicts with `getReadRegion`,
preserving compiler diagnostics for invalid calls. They remain unrun in tooling.
The compiler task reports positive public completion/reuse and propagation of
the original owned transfer error on interpreter, Cranelift, LLVM, and generated
PHP using the shared native resumable core. The former PHP group `B3603` boundary
has been removed in the current compiler working tree. Tooling behavior and
pinned-compiler integration remain unvalidated; no synthetic group is registered
in tooling.

`begin` consumes the plan on both success and checked failure. Its own
plan-validation and native-setup errors use existing `IoError` with
`IoOperation::Configure` / `IoTarget::TransferGroup`, preserving reason and
system-code details rather than exposing internal setup-phase labels. An
adapter's original owned checked `Error` propagates unchanged, not relabeled
or reconstructed. Failed setup releases the plan's loans, leaves no active
group, and submits no work. Shared group descriptions and source assertions
preserve this approved contract without adding a synthetic `begin` declaration.

Deadline preserves exact past instants rather than
collapsing them into an expired marker, so distinct past deadlines remain
distinguishable. A negative timed wait expires only the wait; it does not cancel
the underlying transfer. No other public Time or group operation is specified
by these choices.

[Decision 0137](../../doria/docs/decisions/0137-inherited-device-pending-transfer-helpers.md)
permits lazy, group-capacity-bounded native-only blocking helpers for pending
transfers through inherited synchronous standard devices when native completion
or safe blocking-mode control is unavailable. Helpers retain the real handles,
stable request records, and original buffers, without hidden buffer copies or
per-transfer wrapper allocation. Adapter preparation/completion, Doria execution,
and PHP-runtime-affined callbacks remain on the driving thread and retain their
nonblocking requirements. Ordinary synchronous I/O and idle watches start no
helpers; this is not a general worker model or an exception for blocking Doria
callbacks. Cancellation requests all pending operations before draining any;
loans and device claims remain live until actual host retirement, which can
delay cancellation/drain when the host cannot interrupt an operation. Wait-only
expiry still leaves that transfer active. Public execution remains unverified.

Decision 0110's **Pending Adapter Capabilities** appendix fixes
`PendingReadableStream` and `PendingWritableStream`, extending the corresponding
synchronous transfer interface and `ReadinessStream`. The compiler owns their
nominal declarations, inherited members, writable preparation/completion
receivers, and checked effects. Tooling recognition uses those same inventories;
it does not infer pending support from a readiness source or a synchronous
`read`/`write` method. Existing synchronous adapters keep their contract.

`ReadRequest` and `WriteRequest` are opaque call-scoped borrowed group-slot
views. Preparation accepts a writable request; completion accepts a readonly
request and returns `?ReadOutcome` or `?WriteOutcome`. Their readonly `outcome`
getter has the same nullable result and preserves checked failure, while writable
`complete(outcome)` accepts a nonnullable actual outcome and configures an
immediate preparation without retiring a live native request. Requests cannot
be constructed, consumed, retained, or returned beyond the call. Both phases,
including Doria helpers and cleanup, must remain nonblocking. These checks and
scoped-borrow facts belong to the compiler, not client-side name matching.
Safe helper return loans inferred from a borrowed parameter, including generic
identity helpers, remain usable within that call; local aliases and nonescaping
callbacks do not gain independent request ownership.
Binding hovers combine canonical compiler types, its pending-request identity
predicate, and readonly/writable binding ownership to display **Call-Scoped
Request View**, including local aliases. Ordinary user types with the same short
name do not acquire this contract. This is distinct from returned-value loans
ending at last use or cleanup; no return-loan lifetime is invented for a request.
Preparation precedes native submission; exactly-once completion dispatches
through the complete original adapter after retirement, preserving any actual
progress that wins cancellation.
`null` means no transfer outcome exists after preparation and cancellation or
setup failure before submission. Lack of native submission alone does not imply
`null`: buffered or in-memory immediate results remain actual outcomes.
`Data(0)`, `Accepted(0)`, `WouldBlock`, EOF, and checked failure with progress
remain distinct from absence. Preparation still returns `void`; synchronous
`read`/`write` returns and the ordinary outcome enum cases are unchanged, with no
`NotSubmitted` case added.
The shared pending-adapter fixture and recognition/diagnostic regressions are
prepared against this contract. The compiler task reports native and generated
PHP group execution passing; tooling has not run compiler-matched validation.
Source regressions no longer require the removed `B3603` execution fence for
valid pending adapters or request operations. `E0771` identifies blocking preparation
or completion, including transitive work. `E0772` rejects request ownership and
escape; an already-invalid taking capture can retain `E0645`. These diagnostics
are not suppressed.
Selected storage must be retained by the transfer group or owned by the complete
registered adapter. `E0773` rejects a local buffer that would end with preparation;
`E0774` rejects accessing, transferring, or replacing selected storage before
native retirement. A region descriptor's last use and `request->complete(...)`
do not retire that storage loan. The LSP preserves the compiler's primary range
and related storage-origin or selection location; it does not invent automatic
lifetime-changing fixes. Unrelated adapter state remains independently usable.

Canonical I/O enum fallback documentation derives its cases and payload fields
from compiler declarations, including `IoOperation::Close`, `Register`,
`Unregister`, `Wait`, `Configure`, `Access`, and the approved appended `Seek`,
`SynchronizeData`, `SynchronizeMetadata`, and `Resize`,
and `IoTarget::ReadinessSet` and
`TransferPlan`, and the appended payload-free `TransferGroup`, rather than
keeping a separate operation inventory. No client-owned case is injected.
These facts also supply
completion documentation.
Readiness infrastructure uses the existing ambient `IoError` contract and portable
reasons, not a separate error family. Unknown byte progress remains `null`;
registration and waiting do not invent transfer progress or replace an original
adapter transfer error. Enum exhaustiveness remains compiler-owned, including
the new cases. These local-source regressions do not establish public readiness
execution or an updated production compiler pin.

Transfer-plan configuration failures use the existing `IoError` family with
`IoOperation::Configure` and `IoTarget::TransferPlan`, not a new error type.
The shared error fixture checks ordinary imports and aliases, catch properties,
and nullable byte progress. Andrew subsequently approved `IoOperation::Access`
for borrowed lookup failures on 2026-10-10; it follows `Configure` in the compiler
inventory. Tooling derives that case too, without a client-specific error model.
Borrowed lookup execution and current-source LSP validation remain pending.

Compiler-proved writable returned borrows may be retained in `let writable`
locals without gaining ownership. The same rule applies to ordinary functions,
methods, generic calls, and aliases; readonly borrows cannot be upgraded and
live loans still protect the source. A `writable borrowed get` permits receiver
mutation but returns a readonly borrow. The shared fixture reads that result;
an explicit E0478 negative case rejects binding it as writable. Shared fixtures
also cover a registered-stream local calling `read`. Diagnostics and member
access remain compiler-owned, with no editor-specific exception. This source-only
coverage is prepared; execution against the next compiler checkpoint is pending.

Decision 0110 assigns standalone clock-read failures to
`Doria\Std\Time\ClockError`, not clock-specific `IoOperation` or `IoTarget`
cases. The compiler's ordinary `ClockError implements Error` declaration has
`__construct(string $message, ?int $systemCode = null)` and readonly stored
`string $message` and `?int $systemCode` facts; an absent system code (`null`)
is distinct from a reported zero. Import or qualification is required, not a
prelude alias. Nominal, constructor, and member help uses compiler metadata;
compiler-known constructors share the class-call signature and named-argument
path, without replacing class hover or inventing an authored definition.
Qualified incomplete-code help uses the compiler declaration text, with no
invented source definition. A clock query that fails inside a readiness wait
still uses its `IoError` wait contract. Both preserve the original platform
failure facts through the shared clock implementation. The compiler now declares
`Deadline::fromNow(Duration $duration): Deadline throws ClockError`. It samples
the monotonic clock exactly once inside the factory and preserves exact past
instants for negative durations, including the full signed interval range.
Other clock and group operations remain pending. Source-matched
tooling regressions pass locally for ClockError and the I/O constructors,
including fully qualified names and imported aliases. This does not establish
an updated production compiler pin or installed editor artifact.

The compiler identifies the exact canonical `Doria\Std\Time\Duration` and
`Doria\Std\Time\Deadline` as inline Copy values, including their nullable forms.
Nominal and binding hover describes copying without heap ownership; assignments
and calls leave the original usable. Completion classifies these as value types,
not ordinary heap classes, and does not offer their opaque internal constructors
as public call signatures. Qualified names and imported aliases retain compiler
identity; ordinary user classes with the same basename remain classes. No new
keyword or prelude alias is introduced.
Deadline type and binding hover also explains that expiry preserves the exact
instant, including distinguishable past instants. When a Deadline bounds
completion observation, expiry stops only the wait, not the underlying transfer.
Factory hover, completion, signature help, and named arguments use the existing
compiler-member metadata path for the public `fromNow` declaration; its new
regressions pass against the matching local compiler on macOS. Opaque internal
constructors remain unavailable as public call signatures.
These representation-specific tooling regressions pass locally against the
matching compiler source. This does not establish an updated production compiler
pin or installed editor artifact.

Andrew's numeric ruling defines Duration as signed `int64` nanoseconds; negative
intervals are valid. Duration hover retains its distinct nominal Copy identity
and describes that domain without assigning numeric or wait semantics to
Deadline. The compiler now declares
`Duration::fromNanoseconds(int64 $nanoseconds): Duration` and a readonly
`int64 $nanoseconds` getter. Member completion, hover, signature help, and named
arguments consume those compiler facts; the factory/getter do not query the
clock or add checked effects. Semantic signatures use the compiler's canonical
`int` spelling for its exact `int64` alias, not a narrower integer. The shared
signed-Duration fixture covers both
signed endpoints, negatives, zero, Copy reuse, aliases, and nullable projection.
Numeric-domain and member regressions pass locally against matching compiler
source; compiler diagnostic forwarding also covers forbidden implicit conversions
and writes to the getter. This does not establish an updated production compiler
pin or installed editor artifact. Andrew's separate negative-expiry ruling treats
a negative Duration used for a timed wait or deadline construction as already
expired, not invalid, indefinite, or an immediate poll. This policy does not
advertise undeclared public Time or group operations. Generic `Equatable`
conformance and arithmetic remain unadvertised.

The shared Wait fixture covers all four declared cases, imported aliases,
qualified names, named `duration` and `deadline` payload arguments, Copy reuse,
nullable values, and payload matching. Enum case help consumes compiler payload
types and Copy capability through the ordinary enum path, not a Wait-specific
signature inventory. These regressions pass against the matching local compiler
on macOS; this does not establish an updated production pin or installed editor
artifact. Its new Deadline factory cases cover caught `ClockError`, nullable
Copy reuse, qualified and imported names, zero and negative durations, and the
minimum signed interval. Factory signature and diagnostic regressions cover
named arguments, nominal return type, checked effects, wrong argument types,
and unhandled `ClockError`; these additions pass source-matched validation on
macOS. No production compiler pin or installed-tool refresh is established.

The verified factory/Wait integration used compiler fingerprint
`dev-a20b4000cfba70f0fe73df86b514ff87f286b223e23c11191c85c9ac5d15d7c2`:
390 Rust tests and 61 VS Code tests passed, alongside strict Clippy, formatting,
the local-source build, and shared highlighting checks. Linux/Windows execution
and IntelliJ host tests were not run. Later documentation-only fingerprint
changes do not represent a new tested implementation checkpoint.

The subsequent registration-key integration used compiler fingerprint
`dev-2f9cb02d939cd2c75cd3a2bece3813a29220360221fffad9124dd79a2fb2f110`:
393 Rust tests passed on macOS, along with strict all-target Clippy, formatting,
the local-source build, shared highlighting checks, and 61 VS Code tests. The registration-key fixture also
tokenized without rejected syntax. This does not update the production compiler
pin or installed tools; Linux/Windows execution and IntelliJ host tests remain
unrun.

`IoError::$transferredByteCount` is readonly `?int`. A reported zero is distinct
from `null` (no byte-progress fact); property completion and hover use the same
compiler member metadata as the other I/O error properties.

The exact canonical `Doria\Std\Io\IoError` and
`Doria\Std\Io\InvalidUtf8Error` effects are ambient. They remain catchable and use
the checked runtime transport, but ordinary callables do not need to declare or
catch them. Required nonambient effects still produce compiler-owned diagnostics
when they are neither caught nor declared. When the selected program entrypoint
omits `throws`, the compiler infers its exact escaping set; the server neither
infers effects nor suppresses diagnostics.

Builtin I/O hover keeps ambient effects out of the source signature. For example,
`read_line(string $prompt = ""): ?string` is followed by the two exact ambient
error identities as supplementary runtime information. A checked error may escape
a source `finally` block and supersede a pending nonfatal outcome; a sibling catch
on the same `try` does not catch that finalizer error, while an enclosing catch may.
`E0632` is historical and reserved rather than a live editor diagnostic.
`R1000` remains a runtime outcome for an error that escapes the entry point, so it is
not published as an editor diagnostic.

Stage 46 adds `Doria\Std\Term` under Decision 0140. The module is authored in
Doria, so its classes, enums, members, and case payloads reach hover,
completion, and signature help through the same compiler declarations as any
other module; the server adds no terminal-specific symbol table. Its four
inline Copy values (`Size`, `Position`, `KeyModifiers`, `Style`) are compiler
`InlineValueKind::Terminal` kinds, and hover adds one description for each
together with the shared non-owning inline note. `Doria\Std\Io\WatchKey` and
`WatchSource` are inline kinds too. A `WatchKey` hover says that the key names
one watch in a transfer plan and not the watched source, and repeats the
key-lookup failure contract that read and write keys carry. A `WatchSource`
hover says that a program passes the source itself to `TransferPlan::watch` or
`andWatch` and has no use for the value. `TransferCompletion` hover describes
`Ready(WatchKey $key)`: the source has something the program can take without
waiting, the program asks the source, then calls `resubmitWatch` to watch
again. The standard-stream hover names the readonly `isInteractive` property.
The fixture `editors/fixtures/stage46-terminal.doria` holds a program that
enters raw mode, watches it in a transfer group, and reads keys; the server
reports no diagnostics for it, and a hover regression covers the four terminal
values and `WatchKey`.

The server never guesses a symbol from spelling alone. Ambiguous or unresolved
identifiers return no semantic hover.

## Signature help

`textDocument/signatureHelp` uses the same compiler-resolved callable records as
hover. Free functions, instance methods, static methods, and constructors show
parameter and return types together with any declared `throws` entries in source
order. The server does not reconstruct or alphabetize the compiler's checked-error
effects. Constructor signatures preserve authored `internal`, `override`, and
`parameter` roles. A clause-free `main` hover remains clause-free rather than synthesizing
the compiler's inferred effective set as source syntax.

## PHPDoc presentation

Doria documentation comments use PHPDoc syntax:

```doria
/**
 * Creates a greeting.
 *
 * @param string $name Person to greet.
 * @return string The greeting.
 */
function greet(string $name): string
{
    return "Hello, {$name}!";
}
```

Hover preserves the summary and body text and presents `@param`, `@return`, and
`@throws` tags in dedicated Markdown sections. Unknown tags remain visible instead
of being discarded.

Both editor clients request documentation-comment generation from `doria-lsp`.
Pressing Enter after `/**` creates a PHPDoc-compatible block for the next compiler
declaration and pre-fills the tags that declaration requires: `@template`,
`@param`, `@return`, source-ordered `@throws`, and `@var`. Parameter tags preserve
Doria's `internal`, `take`, and `writable` modifiers. The server reads these facts
from the compiler AST; the VS Code and JetBrains clients do not maintain their own
signature parsers. Ordinary `/*` comments receive the same leading-asterisk block
shape but never receive semantic tags.

## Ownership boundary

The compiler remains the semantic authority. The server builds an IDE-oriented
snapshot from `doriac` parser and semantic-analysis results, maps byte spans to LSP
positions, caches the snapshot by document version, and renders protocol Markdown.
It does not infer a parallel Doria type system.

Native behavioral declarations use the same boundary. In development sources,
the compiler provides typed suite/test facts and exact call, description, and
body spans; Baton/compiler project data provides source scope. The server uses
those facts for declaration and description semantic tokens without parsing
`describe`, `it`, or `test` itself and without treating test-looking paths as
semantic evidence. Slice 2 additionally projects compiler-owned expectation
symbols, matcher members, diagnostics, and automatic `TestAssertion` effects;
it does not reconstruct matcher types or effects. Slice 3 consumes the compiler's
typed matcher candidates and matcher specifications to render concrete actual and
operand types, collection families, stable complexity, invocation mode, and
`TestAssertion` behavior. Testing navigation uses authored compiler ranges and
hierarchy while hiding generated callables and refusing unsafe edits.

The server submits Baton-supplied project sources to one compiler-owned complete
graph per project, with unsaved open text overlaid by canonical path. When Baton
is unavailable, currently open documents use the retained partial graph per
synthetic workspace package; self-contained programs with `main` receive isolated
packages based on compiler syntax and symbol facts. Namespace and imported-symbol
hover supplements the resulting rich semantic facts with canonical qualified names, explicit
aliases, edition-prelude provenance, or compiler-known provenance without
exposing synthetic package IDs. Definition and references use canonical identity
across supplied documents. Explicit alias rename is local to that file; canonical
rename edits direct unambiguous editable occurrences while preserving
explicit aliases, and declines for duplicates, implicit aliases, or incomplete
inputs. Completion offers current-namespace declarations, explicit aliases,
prelude/compiler-known names, and qualified supplied declarations without
treating every dependency short name as imported.

The same compiler-owned global-symbol facts drive the `Use import for ...` code
action. It handles qualified references and unresolved short names matched by
role against declarations in the workspace symbol graph. It reuses an existing
alias, presents ambiguous canonical targets as separate actions, and declines
when introducing the short name would collide. The resulting import section
groups class-like symbols before functions and constants, sorts each group by
canonical qualified name, and preserves comments by refusing to normalize an
interleaved block.

Project discovery is intentionally delegated to Baton. The server consumes its
strict project JSON and complete compiler build plan; it does not read manifests,
locks, or scan directories. Supplied unopened and generated sources participate
in hover and navigation. Generated and Git-cache sources are readonly, so rename
and machine edits are declined when any target is not editable. Partial graphs
remain available for standalone files and discovery failure.

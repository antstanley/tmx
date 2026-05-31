# TMX in the task/workflow-runner landscape

> A research comparison of TMX against existing task runners, CI/CD platforms,
> orchestration engines, build systems, automation tools, and LLM-eval harnesses —
> and an honest read on where someone would actually choose `tmx`.
>
> _Status:_ TMX is an **early-stage spec with no runtime yet** (see [`README.md`](../README.md)
> and [`SCHEMA.md`](./SCHEMA.md)). Everything below compares the *design* against tools that
> are shipping today. Survey date: **2026-05-31**; covers **TMX 0.2.0** and 52 tools across 7
> families.

> **What 0.2.0 changed in this comparison.** TMX 0.2.0 adds three things *on paper* — `map`
> (bounded, single-process fan-out, including a basic single-axis model matrix), `eval` (a real
> measurement verb with model-graded, deterministic, and custom scorers feeding a thresholded
> scorecard), and `produces` (an optional author-declared JSON Schema for typed outputs with
> lint-time checking). These retire three formerly-blunt claims: *"sequential-only with no
> fan-out,"* *"just static Vitest matchers with no model-graded rubrics,"* and *"wholly untyped
> dataflow."* What remains true is the decisive part: TMX is still a **spec with no runtime** — no
> caching, durability, scheduling, distributed parallelism, experiment tracking, results UI,
> dataset versioning, or red-teaming — and an unproven provider abstraction. So every *"stick with
> X"* reason that rests on a shipping engine survives intact. Read every *"TMX can"* below as *"the
> spec describes,"* not *"you can run."*

---

## 1. What TMX is (the thing being compared)

A **Flow** = optional `environment` → optional `context` → required `tasks`. A **Pipeline** is
a Flow's live runtime state. TMX's design has six defining traits; keep them in mind because
every comparison below turns on them:

1. **Multi-format, one model.** Authored in YAML / JSON / JSONC / TOML, all parsing to a single
   JSON model (optional `kind` discriminator).
2. **JSON-state dataflow.** A single Pipeline JSON object is threaded through tasks. Each task
   consumes the state and returns JSON that is **merged back under the task's name**
   (`state[name] = output`); later tasks read it via `${{ tasks.NAME.field }}`. Non-JSON output
   is wrapped (`{message}` / `{blob}`).
3. **Sequential-by-default minimalism.** Tasks run strictly top-to-bottom. The only control flow
   is a per-task `if` skip **plus the bounded iteration of the `map` task** (see 0.2.0 below).
   Still **no branching, no DAG/`needs`, no loops-until-condition, no unbounded/distributed
   parallelism**.
4. **Batteries-included built-ins.** `exec`, `run`, `fetch` (HTTP), `file`, `store` (S3),
   `chat-completion` (LLM, **LLM-native**), `assert` (Vitest matchers), `map` (fan-out),
   `eval` (**eval-native** — see below), `flow` (import a Flow as a task = composition).
5. **Per-task opt-in secrets.** Secrets are auto-masked everywhere; a task gets a secret in clear
   text only if it names it in `secrets`. Lifecycle hooks: `create`/`change`/`destroy`/`error`.
6. **Portable environment substrate.** An `environment` declares *where* a Flow runs
   (local / aws / gcp / azure / fly; container / vm / microvm / process), materialised by a
   pluggable **Provider** (a binary or a Flow with `bootstrap`/`deploy`/`clean`/`destroy`).
   Plus typed `inputs` and `${{ }}` interpolation. Not tied to any hosted platform.

**Added in 0.2.0** (spec-level — like the rest of TMX, these describe behaviour but cannot yet be
executed, since there is no runtime):

- **`map`** — a bounded fan-out task: run an inner task or imported flow once per element of
  `items` (an array or `${{ }}` expression), binding each as `${{ item }}` (alias via `as`), with
  optional bounded `concurrency` and `continueOnError`, collecting an ordered array under the task
  name. The only non-sequential construct; the surrounding task list still runs strictly in order.
- **`eval`** — a measurement task (distinct from `assert`'s boolean gate): score a `subject` over
  an optional `dataset` using `scorers` of three kinds (deterministic `matcher`, model-graded
  `llmRubric` LLM-as-judge, custom `exec`/`run`), weighted per case, with a `threshold` policy,
  emitting a scorecard (`{cases, summary{mean,weightedMean,passRate,p50,p90,count}, passed}`).
- **`produces`** — an optional, author-supplied JSON Schema for a task's output, *specified to
  enable* static linting and autocomplete of downstream `${{ tasks.NAME.field }}` references plus
  an optional runtime conformance check. Purely declarative, per-task, no execution effect.

Stated use cases: **CI/CD, testing, evaluations, configuration, deployment.**

---

## 2. The landscape at a glance — seven families

| Family | Examples | What they optimise for | Relationship to TMX |
| --- | --- | --- | --- |
| **A. Local / dev task runners** | Make, Just, Task, npm scripts, Mage, Rake, Invoke, tox/nox | Running named commands locally; some do file-based incremental builds | TMX's most direct "replace my Makefile" competitors — but none thread structured JSON state |
| **B. Hosted / self-hosted CI/CD** | GitHub Actions, GitLab CI, CircleCI, Azure Pipelines, Buildkite, Jenkins, Drone, Travis | Event-driven build/test/deploy tied to a VCS/platform | TMX overlaps on CI/CD but is platform-independent and sequential-with-bounded-`map`-fan-out (no DAG, no distributed matrix) |
| **C. Cloud / container-native build** | AWS CodeBuild, CodePipeline, Google Cloud Build, Dagger, Earthly, Tekton, Concourse | Managed/containerised reproducible builds | Dagger is TMX's closest *portability* cousin |
| **D. Orchestration & durable execution** | Airflow, Prefect, Dagster, Temporal, **Step Functions**, Argo, Inngest, Restate, Snakemake, Nextflow | Scheduled DAGs, data pipelines, crash-proof long-running workflows | **Step Functions is the closest analogue to TMX's JSON-state model** |
| **E. Monorepo build systems** | Bazel, Buck2, Nx, Turborepo, Gradle, Pants | Content-addressed caching + incremental builds at scale | Orthogonal — TMX is explicitly *not* a caching build system |
| **F. Config / shell / low-code** | Shell, Ansible, n8n, Zapier, Make.com | Glue scripting, idempotent config, SaaS integration | **n8n / Make.com pass JSON between nodes** — conceptual cousins; TMX targets developers |
| **G. AI / LLM eval & agent runners** | promptfoo, OpenAI Evals, LangChain/LangGraph, DSPy, Braintrust, Vitest/Jest, Inngest AgentKit, Mastra | LLM evaluation, agent orchestration | TMX 0.2.0 has a real `eval` verb (matches the shape); promptfoo/Braintrust still far deeper operationally; TMX's edge is *eval-as-one-stage-of-a-pipeline* |

---

## 3. The comparison axes that matter

These are the dimensions on which the choice actually turns:

1. **Authoring & format** — declarative config (YAML/JSON/TOML) vs code-as-pipeline (Go/Python/TS); single-model multi-format vs one DSL.
2. **Control-flow expressiveness** — sequential + `if` skip + **bounded single-process `map` fan-out** (TMX 0.2.0) vs DAG / parallel / branch / loop / multi-axis matrix / retries. TMX's `map` is the only non-sequential construct; there is still no general branching, DAG/`needs`, or distributed/unbounded parallelism.
3. **State & data passing** — accumulating JSON merged-by-name, now **optionally typed per task via `produces`** (TMX) vs flat key=value (GitLab dotenv, GHA outputs) vs **language-enforced** typed objects (Dagger, Flyte, Dagster, Mastra) vs files/artifacts (Make, Bazel) vs none. `produces` is opt-in, author-supplied, and lint/optional-runtime only — not language-level inferred or enforced.
4. **Where it runs & portability** — local / hosted SaaS / self-hosted / runs-anywhere; VCS-coupled vs agnostic; single-binary vs server+workers.
5. **Environment / cloud provisioning** — none vs container-per-step vs Kubernetes-native vs pluggable multi-cloud provider (TMX's distinctive but unproven claim).
6. **Caching & incrementality** — content-addressed hermetic (Bazel/Buck2/Pants/Nx/Turborepo/Gradle) vs file-timestamp (Make/Snakemake) vs **none (TMX)**. Unchanged in 0.2.0: `map` results and `eval` scorecards would recompute every run.
7. **Built-in task vocabulary** — generic shell only vs rich typed built-ins (HTTP/file/store/LLM/assert/map/eval) — TMX's batteries-included angle.
8. **LLM / eval nativeness** — dedicated harness *depth* (promptfoo, Braintrust, OpenAI Evals, LangSmith) vs a real eval *verb* (TMX 0.2.0 `eval`: model-graded + matcher + custom scorers → thresholded scorecard) vs LLM-as-a-step (Dagger, n8n). TMX now matches the canonical `Eval(dataset, subject, scorers)→scorecard` *shape*; the gap to the dedicated tools is operational (tracking, UI, dataset versioning, red-teaming) — and on paper, since nothing runs.
9. **Secrets** — ambient env (most) vs encrypted store + masking (CI platforms, Ansible Vault) vs **per-task opt-in unmasking (TMX)**.
10. **Composition & reuse** — import/include, modules, reusable workflows/orbs/plugins, registries vs TMX flow-as-task.
11. **Durability / fault-tolerance** — replay/journal (Temporal, Restate, Inngest) vs **none (TMX)**.
12. **Typed inputs & interpolation** — typed flow inputs + `${{ }}` (TMX, Kestra, reusable workflows) vs untyped env.
13. **Maturity & ecosystem** — production-proven with plugin ecosystems vs **early-stage spec, no runtime (TMX)**.

---

## 4. Family-by-family comparison

Each table's key column for TMX is **"How data passes between steps."** That is TMX's signature,
so it is what most cleanly separates it from each tool.

### Family A — Local / developer task runners

| Tool | Authoring | Control flow | How data passes between steps | Where it runs |
| --- | --- | --- | --- | --- |
| **GNU Make** | Makefile DSL | Prerequisite **DAG**, `-j` parallel | Filesystem (timestamps) + flat string vars | Local (Unix) |
| **Just** | justfile DSL | Prereq DAG, sequential recipes | Filesystem / exit codes / string vars | Local, cross-platform single binary |
| **Task (Taskfile)** | YAML | Parallel `deps` + sequential `cmds`; `for`; up-to-date checks | Filesystem + **string** vars (`sh:`/`ref:`) — no JSON accumulation | Local, single binary (+remote Taskfiles) |
| **npm/pnpm/yarn scripts** | JSON map | pre/post hooks; `&&`; workspace fan-out | Filesystem / env (`npm_*`) | Local/CI, Node ecosystem |
| **Mage** | Go code | `mg.Deps` parallel / `SerialDeps` | Go vars / files | Local, Go toolchain |
| **Rake** | Ruby DSL | Prereq DAG, `multitask` parallel | Ruby globals / files | Local, Ruby |
| **Invoke** | Python code | pre/post chains, sequential | Python vars; `Result` objects | Local (+SSH via Fabric) |
| **tox / nox** | INI / Python | Per-virtualenv matrix | Filesystem / env (isolated venvs) | Local/CI, Python only |

**Takeaway:** This is the family TMX most directly pitches against ("replace your Makefile/Bash").
They are mature, zero-to-low-install, and ergonomic. **None of them threads a structured JSON
state object between steps** — they pass data through the filesystem, exit codes, and flat string
variables. Make/Task/Rake also do file-timestamp incrementality, which TMX does not. TMX's bet
here is: same "list of tasks" simplicity, but with typed inputs, merged-JSON dataflow, masked
secrets, multi-format authoring, and a path to running the same file in the cloud.

### Family B — Hosted / self-hosted CI/CD

| Tool | Authoring | Control flow | How data passes between steps | Where it runs |
| --- | --- | --- | --- | --- |
| **GitHub Actions** | YAML | Job **DAG** (`needs`), `matrix`, step `if`, `continue-on-error` | env / `$GITHUB_OUTPUT` step outputs / upload-download **artifacts** / cache | GitHub-hosted + self-hosted runners; **VCS-locked to GitHub** |
| **GitLab CI/CD** | YAML | Stages + `needs` DAG, `parallel:matrix`, `rules`, `retry` | **artifacts** + `dotenv` reports (**flat KEY=VALUE strings, no nesting**) | GitLab.com + self-managed runners |
| **CircleCI** | YAML | Workflow DAG (`requires`), `matrix`, test-splitting `parallelism` | **workspaces** (files) / caches / artifacts | Cloud + self-hosted runners; multi-VCS |
| **Azure Pipelines** | YAML (+Classic) | stages→jobs→steps DAG (`dependsOn`), `matrix`, approvals | output variables (flat strings) / pipeline **artifacts** | Microsoft-hosted + self-hosted agents |
| **Buildkite** | YAML/JSON or **dynamic** (script emits steps) | `wait` barriers, `depends_on` DAG, `matrix`, retries | `buildkite-agent meta-data` (flat **key=value**) / artifacts | Hosted control plane + **your own agents** (secrets never leave your perimeter) |
| **Jenkins** | Groovy (Jenkinsfile) | `parallel{}`, `matrix{}`, `when{}`, full Groovy | Groovy vars / `stash`/`unstash` (files) / archive | **Self-hosted** controller + agents |
| **Drone CI** | YAML | Steps run **sequentially within a pipeline**; `depends_on` is **pipeline-level** (multi-pipeline DAG); `when` | Shared **workspace volume** (files) | Self-hosted (now Drone by Harness) |
| **Travis CI** | YAML | Stages, build matrix | Caches / artifacts | Hosted SaaS — **effectively legacy** since the 2020–21 OSS-credit changes |

**Takeaway:** This family *is* CI/CD. TMX 0.2.0's `map` means the old "TMX *structurally cannot*
do matrix fan-out" line is retired — a `map` over a list of inputs/model-configs expresses a
**basic single-axis matrix** (nest for more axes). But at scale they still win decisively: a true
multi-axis `matrix:` cross-product, **distributed runners across machines**, a job DAG (`needs`),
marketplaces, and a real engine that runs it — whereas TMX's fan-out is single-process, bounded,
and unexecutable spec. They are also **coupled to a platform/VCS** and pass data through
**files/artifacts and flat key=value outputs** — none has TMX's single nested-JSON Pipeline merged
under each task name (GitLab's `dotenv` is the closest, and it is flat strings only). TMX's pitch
remains **portability and locality**: one file that runs identically on a laptop, in any CI, or on
any cloud, with no control plane.

### Family C — Cloud / container-native build

| Tool | Authoring | Control flow | How data passes between steps | Where it runs |
| --- | --- | --- | --- | --- |
| **AWS CodeBuild** | YAML `buildspec` | Fixed phases (install→pre_build→build→post_build); batch fan-out | S3 **artifacts** / `exported-variables` (strings) / cache | AWS-only managed compute |
| **AWS CodePipeline** | JSON/YAML | Sequential **stages**, parallel actions, approvals, rollback | Named S3 **artifacts** + scoped variables | AWS-only managed orchestrator |
| **Google Cloud Build** | YAML | Step **DAG** via `id`/`waitFor` | Shared `/workspace` volume + substitutions | GCP-only (managed/private pools) |
| **Dagger** | **Code** (Go/Python/TS mature; PHP/Java/.NET/Elixir/Rust community/experimental) | Full host-language control flow; auto-parallel **DAG**; content-addressed cache | **Typed objects** (Container/Directory/File/Secret) chained through the DAG | **Anywhere** — local, any CI, any cloud; engine in a container |
| **Earthly** | Earthfile DSL | Target DAG; `IF`/`FOR`/`WAIT`; auto-parallel | `SAVE ARTIFACT` / `COPY +target/file` (files/images) | Anywhere via BuildKit — **but vendor wound down Cloud/maintenance in 2025** |
| **Tekton** | K8s YAML (CRDs) | Task DAG (`runAfter`), `when`, `matrix`, `finally` | **Workspaces** (PVC files) + small string `results` | **Any Kubernetes** |
| **Concourse CI** | YAML | `in_parallel`, `across` (matrix), `try`, hooks; resource-driven | External **Resources** (git/S3/registry) + file dirs | Self-hosted (ATC + workers) |

**Takeaway:** **Dagger is TMX's closest philosophical cousin** — "write your pipeline once, run it
identically anywhere," and it even shipped LLM/agent primitives in 2025. The difference is
authoring and engine: Dagger is *imperative code* building a lazily-evaluated **typed DAG** with
**content-addressed caching and automatic DAG parallelism** and a real shipping engine; TMX is
*declarative multi-format config* running sequential tasks (with one bounded `map` fan-out and
optional `produces` typing in 0.2.0) over a single merged-JSON Pipeline, and (so far) **no engine,
no caching, no DAG parallelism**. 0.2.0 narrows the "zero parallelism, fully untyped" gap slightly
but does not close it. If "portable programmable CI" is the goal, Dagger does it today with far
more power; TMX's counter-bet is *declarative simplicity* and built-in
`fetch`/`store`/`chat-completion`/`assert`/`map`/`eval` task types.

### Family D — Orchestration & durable execution

| Tool | Authoring | Control flow | How data passes between steps | Where it runs |
| --- | --- | --- | --- | --- |
| **Apache Airflow** | Python | Scheduled DAG, branching, dynamic mapping, sensors, retries | **XCom** — small keyed values via metadata DB (large data → external storage) | Self-hosted / MWAA / Composer / Astronomer |
| **Prefect** | Python | Dynamic runtime DAG, `.map()`, retries, caching | Python objects + optional persisted **Result** | Self-hosted / Prefect Cloud |
| **Dagster** | Python | Asset graph, dynamic outputs, schedules/sensors | **I/O Managers** persist outputs (S3/warehouse); typed lineage | Self-hosted / Dagster+ |
| **Temporal** | Code (Go/Java/TS/Py/.NET) | Full language control flow; **durable replay**; retries, signals, sagas | **Durable state = your program's variables**, reconstructed by replaying an event history | Self-hosted cluster / Temporal Cloud |
| **AWS Step Functions** | **ASL JSON** | `Choice`/`Parallel`/`Map`, retry/catch, waits | **★ A JSON object threaded between states; `ResultPath` MERGES a state's result back under a key — directly analogous to TMX's `state[name]=output`** | AWS-managed only |
| **Argo Workflows** | K8s YAML | Steps/DAG, `withItems`/`withParam` loops, `when`, retries | Named **parameters** (strings) + **artifacts** (S3 files); `{{steps.x.outputs...}}` | **Any Kubernetes** |
| **Inngest** | Code (TS/Py/Go) | Durable steps, `waitForEvent`, parallel, flow-control | Memoised step outputs reconstructed in your function | Inngest Cloud / self-hosted; your code runs anywhere |
| **Restate** | Code (TS/Java/Go/Py/Rust) | Durable steps/journal, sagas, awakeables; Virtual Objects | Replay journal **+ embedded durable K/V** state per object | Single self-contained binary / Restate Cloud |
| **Snakemake** | Python DSL (Snakefile) | Pull-based file DAG, checkpoints, retries | **Files** (Make-style input/output matching) | Local → HPC / K8s / cloud |
| **Nextflow** | Groovy DSL | **Channel dataflow**, implicit parallelism, `-resume` | **Channels** of items/files between processes | Local → HPC / AWS-GCP-Azure Batch |

**Takeaway:** **AWS Step Functions is the single closest analogue to TMX's defining trait.** Its
`ResultPath` merges each state's output back into a running JSON object — almost exactly
`state[name] = output` — and with 0.2.0 the analogy now extends to fan-out: TMX's bounded `map`
is a spirit-level cousin of Step Functions' `Map` state (and of Airflow dynamic task mapping /
Argo `withParam`). But Step Functions still has **`Choice` branching, `Parallel` branches, a
distributed Map over S3**, managed durability, and 200+ AWS integrations — and it runs today. The
durable engines (Temporal, Restate, Inngest) solve a problem TMX doesn't attempt — *crash-proof
long-running* workflows. The data engines (Airflow, Dagster, Nextflow) are about *scheduled,
cluster-scale, large-data* pipelines. TMX is none of these: a lightweight, single-process runner
whose only non-linear move is a bounded `map`. If you need durability, scheduling, general
branching, or parallel fan-out at scale, these win; if you want a small portable file, they are
massive overkill.

### Family E — Monorepo build systems

| Tool | Authoring | Control flow | How data passes between steps | Caching |
| --- | --- | --- | --- | --- |
| **Bazel** | Starlark | Parallel action **DAG**; `select()` | Content-addressed **artifacts** (CAS) + typed providers | Hermetic content-addressed local+**remote** cache/exec |
| **Buck2** | Starlark (+BXL) | Parallel DAG; hybrid local/remote racing | CAS artifacts by digest | Hermetic, RE-first; Rust core |
| **Nx** | JSON + TS plugins | Parallel task graph; `affected` from git diff; distributed | Cached **file outputs** keyed by computation hash | Computation cache; remote via Nx Cloud (self-hosted cache options churned 2025–26 after a cache-poisoning CVE) |
| **Turborepo** | JSON(C) | Topological parallel; `--filter`; watch | Cached file outputs + logs by content hash | Open Remote Caching spec (self-hostable) |
| **Gradle** | Groovy/Kotlin DSL | Configuration→execution DAG, `--parallel` | Fingerprinted task inputs/outputs (files) | Local + remote build cache, configuration cache |
| **Pants** | BUILD (Python-like) | Parallel rule graph; **dependency inference** | Content-addressed artifacts | Hermetic sandboxed cache; REAPI remote |

**Takeaway:** These are **orthogonal** to TMX. Their reason to exist is **content-addressed caching
and correct incremental rebuilds** of large repos — exactly what TMX's spec says it is *not*. They
pass data as cached file artifacts in a parallel DAG, never as a JSON state object, and have no
shell-scripting ergonomics, LLM/assert primitives, or cloud provisioning. You would never pick
between Bazel and TMX; you might use TMX to *glue around* a Bazel build.

### Family F — Config / shell / low-code automation

| Tool | Authoring | Control flow | How data passes between steps | Where it runs |
| --- | --- | --- | --- | --- |
| **Shell scripts** | Shell source | Full imperative; parallel only via `&`/`xargs -P` | env vars / `$(...)` / pipes / files (**untyped strings**) | Anywhere a shell exists; scripts non-portable across shells |
| **Ansible** | YAML playbooks | `when`/`loop`/`block-rescue`; host-parallel (`forks`/`serial`) | `register` / facts / `set_fact` (host-scoped vars) | Control node → SSH/WinRM to fleets; AWX/AAP |
| **n8n** | Visual graph → **JSON** | Branch/merge/loop graph; per-item iteration; sub-workflows | **★ Array of JSON items** `{json,binary}` node-to-node (`{{ $json.field }}`) | **Self-hosted** or n8n Cloud |
| **Zapier** | Proprietary GUI | Linear actions + `Paths` branches; filters; loops | Field-mapping UI over JSON (no visible state object) | **Hosted SaaS only**; 7,000+ connectors |
| **Make.com** | Proprietary GUI | Routers/iterators/aggregators (richer than Zapier) | **Bundles** (JSON packets) module-to-module | Hosted SaaS only; 1,500+ connectors |

**Takeaway:** TMX explicitly positions as a **shell-script replacement**, keeping shell as a
first-class executor (`exec`/`run`) but wrapping it in typed JSON dataflow, masked secrets, and
portability. The interesting cousins are **n8n and Make.com**: both pass **JSON between nodes**, a
real analogue to TMX's Pipeline — and 0.2.0's `map` (per-element iteration binding `${{ item }}`,
collecting an ordered array) makes the cousinhood closer to n8n's per-item processing. The
differences hold: they are **branching/merging visual graphs** aimed at integration/iPaaS (n8n is
self-hostable; Zapier/Make are hosted-only), authored in a GUI, with huge connector catalogs. TMX
is **text-defined, sequential-with-one-bounded-`map`, developer-facing**, and not a connector
platform. Ansible's `register` is a loose analogue to capturing a task output, but Ansible is
host-fleet config management, not a single-process dataflow runner.

### Family G — AI / LLM eval & agent runners

| Tool | Authoring | What it does | Eval/agent model | Where it runs |
| --- | --- | --- | --- | --- |
| **promptfoo** | YAML (+JS/TS) | LLM eval & **red-team** harness | prompts × providers × tests **matrix**; tiered asserts + model-graded rubrics | Local/CI/library; cloud (OpenAI **announced** acquisition Mar 2026; stays OSS) |
| **OpenAI Evals** | YAML+Python / JSON API | Model benchmarking | Dataset iteration + rich graders (string/similarity/model/tool) | Local OSS / hosted Evals API (OpenAI-centric) |
| **LangChain / LangGraph** | Code (Py/TS) | Agent orchestration | Stateful **graph**: cycles, conditional edges, parallel, durable checkpoints; typed shared state w/ reducers | Anywhere; LangGraph Platform for hosting |
| **DSPy** | Python | LLM programming + **prompt optimization** | Typed Signatures + optimizers that search prompts against a metric | Local/anywhere, many LM backends |
| **Braintrust** | Code (TS/Py/Go) | Eval **platform** | `Eval(data, task, scorers)` → experiments with score diffs; autoevals | Hosted SaaS (self-host enterprise) |
| **Vitest / Jest** | Code (JS/TS) | Test runner | `expect()` matchers — **the source of TMX's `assert` matcher vocabulary** | Local/CI |
| **Inngest AgentKit** | TypeScript | Durable multi-agent networks | Router + shared Network State; durable `step.ai` | Inngest Cloud / self-hosted |
| **Mastra** | TypeScript | AI agent + workflow framework | Typed (Zod) durable workflows: sequential/parallel/branch/loop, suspend/resume | Anywhere Node runs; optional Mastra Cloud |

**Takeaway:** This family tests TMX's "evaluations" use case, and 0.2.0 changes the verdict here
the most. TMX's `eval` is now a **genuine measurement verb** — model-graded `llmRubric`
(LLM-as-judge, 0..1), deterministic `matcher`, and custom `exec`/`run` scorers, weighted per case
over an optional dataset, with a `threshold` policy and a real scorecard — i.e. the same canonical
`Eval(dataset, subject, scorers) → scorecard` **shape** as promptfoo, Braintrust, and OpenAI
Evals, and a basic provider matrix via `map` over model configs. So *"just static Vitest matchers,
no model-graded rubrics"* is retired. **But the gap is now operational, not conceptual — and it is
wide:** no experiment tracking or run-over-run regression diffs (Braintrust/LangSmith core), no
results UI or comparison matrix (promptfoo's zooming grid), no dataset versioning, no large
prebuilt grader/autoevals library (no BLEU/ROUGE/cosine), no red-teaming/security plugins
(promptfoo's 50+), no cost/latency tracking, and no online/production evaluation — and it is
**on paper**, since there is no runtime to compute a single score. DSPy is orthogonal (it
*optimises* prompts against a metric — MIPROv2/GEPA — which TMX never does). TMX's durable edge is
unchanged: the eval is *one stage of a portable `build → call-LLM → eval → assert → deploy`
pipeline* with merged-JSON state, not a standalone harness or optimiser. Today teams bolt promptfoo
onto a separate CI system; TMX *specifies* folding the two together. The agent frameworks
(LangGraph, AgentKit, Mastra) remain the *maximalist opposite* of TMX's control-flow stance and
would more likely be *invoked by* a TMX `exec`/`run` step than replaced by it.

---

## 5. Where does TMX win? — the decision guide

Read this as: **"If you're currently reaching for X, choose TMX when ___; stick with X when ___."**

> Every *"choose TMX when"* below is contingent on a TMX runtime existing — it is a spec today, and
> every alternative ships and runs now. Read the rows as design positioning, not a buy decision.

| If you'd otherwise use… | Choose **TMX** when… | Stick with the alternative when… |
| --- | --- | --- |
| **Make / Just / Task** | You want typed inputs, merged-JSON dataflow (now optionally typed per task via `produces`), masked secrets, multi-format authoring, and a path to the cloud — not just shell strings | You need file-timestamp incremental builds, or just the simplest local command catalog that *ships and runs today* (TMX has no runtime) |
| **Shell scripts** | You want shell steps **plus** reproducible JSON state passing, secret hygiene, assertions, optional typed outputs, and portability | It's throwaway glue, or no runtime can be installed — note TMX has no runtime to install yet either |
| **GitHub Actions / GitLab CI / CircleCI** | You want the *same* pipeline to run on a laptop, in any CI, and on any cloud with no control plane or VCS lock-in, and one bounded `map` axis (over models/inputs) covers your spread | You need a true multi-axis `matrix:` cross-product, **distributed runners**, a job DAG (`needs`), or the marketplace — running on a real engine today |
| **AWS Step Functions** | You want the JSON-state-merge model **plus a bounded `Map`-style fan-out without AWS lock-in**, in a small portable file | You're all-in on AWS and need managed durability, `Choice` branching, `Parallel`/distributed Map, and 200+ native integrations |
| **Dagger** | You prefer a small declarative file over pipeline code, want built-in `fetch`/`store`/`chat-completion`/`assert`/`map`/`eval`, and bounded fan-out is enough — *and a runtime exists* | You want real content-addressed caching, automatic DAG parallelism, pipeline-as-typed-code, and a shipping engine today |
| **n8n / Zapier / Make.com** | You're a developer who wants version-controlled, text-defined, sequential-with-bounded-`map` pipelines (not a GUI/connector platform) | You need a big connector catalog, branching/merge graphs, per-item iteration over arbitrary connectors, or non-engineer authoring |
| **Temporal / Airflow / Prefect** | Your workflow is short and mostly sequential with at most a bounded fan-out step, and you don't need durability/scheduling/parallel scale | You need crash-proof long-running execution, scheduling, backfills, dynamic mapping, or large/distributed parallel DAGs |
| **promptfoo / Braintrust** | The eval is *one stage* of a broader build/deploy pipeline and a self-contained scorecard suffices: model-graded `llmRubric` + matcher + custom scorers, weighted, gated by a `threshold` (a `map`-over-models basic matrix covers your provider spread) | You need experiment tracking, run-over-run score diffs, a results UI, dataset versioning, a large prebuilt grader/autoevals library, or red-teaming — shipping today |
| **Flyte / Dagster** | You want optional typed task outputs (`produces` JSON Schema) with lint-checked `${{ tasks.NAME.field }}` in a tiny declarative file, without adopting a Python/K8s framework | You need language-enforced types, runtime-checked typed lineage, caching keyed on typed signatures, scheduling, and a running engine |
| **Bazel / Nx / Turborepo** | (You don't — they solve a different problem) | You need content-addressed caching and incremental monorepo builds |

### TMX's genuine sweet spot

A developer who wants a **small, dependency-light, vendor-neutral file** to script **CI / eval /
deploy glue** with **structured (optionally typed) JSON data passing**, a **bounded `map`
fan-out**, and a **first-class `eval` gate** — *without* standing up Temporal/Airflow, learning
Dagger's SDK, bolting promptfoo onto a separate CI, or coupling to a platform. The standout
scenario, sharpened by 0.2.0, is **LLM-in-the-loop glue**: `build → map over a dataset → call an
LLM → eval (model-graded) → assert/threshold → upload/deploy`, expressed once in one file. No
single shipping tool bundles *(multi-format declarative + accumulating JSON state + bounded
fan-out + LLM + model-graded eval + opt-in secret masking + portable provider substrate)* in
exactly this way — **but** that bundle is currently a specification, and each individual capability
is matched or beaten by a tool that runs today.

---

## 6. Reality check — where TMX's claims are weak or already solved

An honest comparison has to flag this, because TMX's strongest-*sounding* claims are the most
contested. (Updated for 0.2.0 — `map`/`eval`/`produces` soften two of these, but the decisive ones
hold.)

- **Still a spec with no runtime.** `map`, `eval`, and `produces` add capability *on paper only* —
  every competitor here (CI platforms, Step Functions, Dagger, n8n, Temporal/Airflow,
  promptfoo/Braintrust/OpenAI Evals/LangSmith, Flyte/Dagster) ships and runs today. Read every
  *"TMX can"* as *"the spec describes."*
- **"JSON-state dataflow" is not novel.** `state[name] = output`, read via `${{ tasks.NAME.field }}`,
  is the same merge-by-name model as AWS Step Functions and n8n. New in 0.2.0: `produces` lets a
  task *optionally* attach an author-supplied JSON Schema, enabling lint-time reference checking — a
  typing affordance those tools lack out of the box, but opt-in, per-task, and **not** language-level
  inferred or enforced (an untyped task stays untyped; a wrong schema mislints).
- **Control flow is no longer "sequential-only with zero fan-out," but the change is bounded.**
  `map` is the *only* non-sequential construct — single-axis, single-process, bounded `concurrency`,
  collecting an ordered array — and the surrounding list still runs strictly in order. Still no
  general branching beyond `if`, no DAG/`needs`, no loops-until-condition, and no unbounded or
  distributed parallelism (vs CI distributed runners, Step Functions distributed Map over S3,
  Airflow/Argo cluster fan-out). Multi-axis matrices require nesting `map`s; there is no first-class
  provider-model matrix.
- **TMX now has a real eval *verb*, not just static matchers — but the gap is operational and
  wide.** `eval` specifies model-graded `llmRubric`, deterministic `matcher`, and custom scorers,
  weighted over a dataset, with a `threshold` and a scorecard — the same canonical
  `Eval(dataset, subject, scorers) → scorecard` *shape* as promptfoo/Braintrust/OpenAI Evals, so
  *"just static Vitest matchers"* is retired. What it lacks: experiment tracking, run-over-run
  regression diffs, a results UI, dataset versioning, a large prebuilt grader/autoevals library,
  red-teaming/security plugins, cost/latency tracking, and online/production evaluation — and there
  is no runtime to compute even one score.
- **"Portable programmable CI that runs anywhere" is Dagger's exact pitch** — executed with
  content-addressed caching, automatic DAG parallelism, and a real engine TMX lacks. 0.2.0 narrows
  the parallelism/typing gap slightly; it does not close it.
- **No caching, durability, scheduling, or distributed parallelism.** `map` results and `eval`
  scorecards recompute every run; TMX is still not a substitute for build systems (Bazel/Nx/Turbo),
  durable engines (Temporal), or data orchestrators (Airflow/Dagster).
- **Kestra still occupies the exact niche, unchanged by 0.2.0** — a shipping, YAML-first, typed-I/O
  orchestrator with 1,200+ plugins (see §7). **"Why not Kestra?" remains the hardest question.**
- **The pluggable multi-cloud provider/environment abstraction remains the most ambitious and
  least-proven claim** — 0.2.0 doesn't touch it, and with no runtime there is nothing to demonstrate
  same-flow portability across providers.

**Net:** 0.2.0 makes TMX a *more complete and more credible design* — the `eval` and `map` shapes
are well chosen and retire the bluntest old criticisms — but it is still a design. The verdict is
one of degree, not kind: where TMX once *"structurally could not,"* it now *"specifies how it
would,"* while every competitor here ships and runs today.

---

## 7. Also worth knowing (not in the main survey)

Notable tools a complete picture should include:

| Tool | Why it matters to TMX |
| --- | --- |
| **Kestra** | **The closest conceptual competitor**: YAML-first declarative orchestrator, typed inputs/outputs, downstream output references, 1,200+ plugins, self-hostable. The benchmark TMX must differentiate from. |
| **Windmill** | Self-hosted, code-first (TS/Py/Go/Bash/SQL) workflow engine with explicit step-output passing and ~20ms step overhead — undercuts TMX's "lightweight data-passing runner" claim. |
| **Trigger.dev** | OSS, self-hostable, TS durable background jobs/workflows with LLM/AI task patterns — overlaps TMX's LLM-native + durable ambitions. |
| **Flyte** | Kubernetes-native, **language-enforced typed** inputs/outputs between tasks + caching — a contrast to TMX's now-*optionally*-typed (`produces`) JSON merge, which is opt-in and lint-only. |
| **Hatchet** | Postgres-backed durable execution / DAG workflows — the "durable, self-hostable, data-passing" middle ground. |
| **Woodpecker CI** | Active OSS community fork of Drone — the lightweight container-native CI the Drone entry's "momentum unclear" note points toward. |
| **Garden** | Declarative YAML build/test/deploy for K8s/containers with dev/CI parity — relevant to TMX's "same flow local or cloud" pitch. |
| **Pulumi Automation API** | Programmatic infra provisioning embeddable in a workflow — a real alternative for the "provision the environment, then run tasks" part where TMX is least proven. |
| **dbt** | Declarative DAG where models pass data via the warehouse + `ref()` lineage — the dominant declarative pipeline tool in data. |
| **GitHub composite actions / reusable workflows** | The mainstream analogue to TMX's flow-as-task composition (typed inputs/outputs/secrets) — the composition axis should compare against it. |
| **Sake** | YAML task runner across local **and remote (SSH)** hosts — overlaps TMX's environment-portability for simple cases. |
| **Spin / wasmCloud** | WebAssembly runners offering a different "portable substrate" model (run the same artifact anywhere) — a philosophical alternative to TMX's provider abstraction. |

---

## 8. Bottom line

TMX 0.2.0 is real, narrowly-scoped progress: `map` gives it bounded single-process fan-out (and a
basic single-axis model matrix), `eval` gives it a genuine measurement verb whose model-graded +
deterministic + custom scorers and thresholded scorecard mirror the canonical shape of promptfoo,
Braintrust, and OpenAI Evals, and `produces` gives it an optional author-declared typed-output
contract with lint-time reference checking. Those three retire the doc's bluntest old claims —
*"sequential-only,"* *"static matchers only,"* and *"wholly untyped"* — and move the CI-matrix,
eval-tool, and (new) Flyte/Dagster typed-dataflow rows from *"structurally can't"* to *"can express
the basic case."*

But the comparison's centre of gravity is unchanged:

- TMX is still a **specification with no runtime**. It has no caching, durability, scheduling,
  distributed or unbounded parallelism, general branching/DAG, experiment tracking, results UI,
  dataset versioning, prebuilt grader libraries, or red-teaming — and its headline multi-cloud
  provider abstraction stays unproven because nothing runs to prove it.
- It is **not** a build system (Bazel/Nx/Turborepo), durable engine (Temporal/Restate), data
  orchestrator (Airflow/Dagster), or integration platform (n8n/Zapier) — and shouldn't try to be.
- Its closest analogues are **Step Functions** (JSON-state merge + `Map`), **n8n/Make.com** (JSON
  between steps + per-item iteration), **Dagger** (portable run-anywhere), **Task/Just**
  (declarative local runner), and most pointedly **Kestra** (declarative YAML flow with typed task
  outputs).
- For a skeptical engineer: 0.2.0 is a **more complete, more credible design** — the `eval` and
  `map` shapes are well chosen — but it is still a design. Adopt it as a portable
  `build → map → LLM → eval → assert → deploy` *format* to track and prototype against; reach for
  Kestra, Dagger, promptfoo, or Braintrust when you need something that **executes today**.
- The work to make it real: ship a runtime + conformance suite, prove the provider/environment
  abstraction, and have a crisp answer to **"why TMX over Kestra, Step Functions, and Dagger?"**

---

## 9. Where TMX is the clear choice

Five areas where the **0.2.0 bundle** — one small, portable, declarative file combining
JSON-state dataflow, bounded `map` fan-out, LLM `chat-completion`, model-graded `eval`, `assert`,
opt-in masked secrets, and a portable provider substrate — genuinely beats every alternative, *and*
the missing pieces (caching, durability, distributed scale, eval dashboards) don't bite.

> These presume a TMX runtime ships. They are the spots where the *combination* is both novel and
> unmet by an existing tool — not claims that any single axis beats a specialist.

1. **LLM evaluation as a CI/CD release gate.** `build → map over a dataset → call the model →
   eval (model-graded `llmRubric` + matcher) → threshold → block the merge`, in one checked-in,
   vendor-neutral file that runs identically on a laptop and in any CI. **Clear because** nobody
   else makes the eval *be the gate in the same artifact as build/deploy*: promptfoo/Braintrust are
   separate tools watched in a dashboard, CI platforms have no eval verb, Step Functions isn't local
   or eval-native. _Boundary:_ pass/fail gating on moderate datasets — not leaderboards, experiment
   history, or red-teaming.

2. **Batch LLM / data-generation & grading jobs.** A declarative recipe — `map` over inputs →
   `chat-completion` per item → `store` to S3 → `eval`/score — with masked secrets and typed
   `produces` contracts. **Clear because** it's a reviewable, checked-in, language-agnostic spec for
   the job, versus a bespoke Python script (no structure/secret-hygiene/typing) or n8n (GUI/connector
   platform, not a dev artifact). _Boundary:_ bounded concurrency suits hundreds–thousands of items;
   millions want a data engine.

3. **Vendor-neutral, locally-reproducible CI/CD glue.** One pipeline file that runs the same on a
   laptop, in any CI, and on any cloud — no control plane, no platform-specific YAML, no SDK.
   **Clear because** it's the anti-lock-in play: GitHub Actions YAML is GitHub-only and Dagger needs
   an SDK + engine; the multi-format + portable-provider + locality combination wins for multi-VCS
   shops, OSS projects, and teams who want CI reproducible locally byte-for-byte. _Boundary:_
   linear-ish flows with at most a bounded `map` axis — not distributed matrix builds, marketplaces,
   or job DAGs at scale.

4. **Structured-JSON automation that's outgrown shell but doesn't need an orchestrator.** `fetch` an
   API → branch on the JSON (`if`) → `assert` the response → `file`/`store` the result, passing a
   real accumulating JSON object between steps with `produces` typing and opt-in masked secrets.
   **Clear because** it sits between bash soup (untyped strings, leaky secrets) and standing up
   Airflow/Temporal (massive overkill): the lightweight structured-dataflow runner for glue, with
   pre-run linting of `${{ tasks.x.field }}`. _Boundary:_ short-lived, single-process — no
   durability/replay or scheduling.

5. **An embeddable, validatable task-DSL inside your own product.** Let users (or another system)
   define "flows" using a spec'd, **JSON-Schema-validated**, multi-format task language rather than a
   homegrown DSL or an embedded Temporal/Airflow. **Clear — and uniquely *unblocked by the
   no-runtime gap*** — because here the asset is the specification + schema + conformance and *you*
   bring the executor: TMX offers a clean, minimal, auditable, sandboxable model (sequential +
   bounded `map`, typed inputs/outputs, opt-in secrets, `kind`-dispatch). _Boundary:_ you commit to
   implementing or vendoring execution.

**The through-line:** TMX is the clear choice wherever the win is *one small, portable, declarative
file that bundles dataflow + bounded fan-out + LLM + eval + secrets*, and the workload is light
enough that caching, durability, and distributed scale are irrelevant. The sharpest single wedge is
**#1 (LLM-eval-as-CI-gate)** — the one place the combination is both genuinely novel and genuinely
unmet by a shipping tool.

---

_Sources: vendor documentation and product pages for each tool, surveyed 2026-05-31; eval-tool
depth (promptfoo, Braintrust, OpenAI Evals, LangSmith) re-verified for the 0.2.0 refresh. TMX
details from this repo's [`README.md`](../README.md) and [`SCHEMA.md`](./SCHEMA.md), at spec
version **0.2.0**._

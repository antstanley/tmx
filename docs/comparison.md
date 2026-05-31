# TMX in the task/workflow-runner landscape

> A research comparison of TMX against existing task runners, CI/CD platforms,
> orchestration engines, build systems, automation tools, and LLM-eval harnesses —
> and an honest read on where someone would actually choose `tmx`.
>
> _Status:_ TMX is an **early-stage spec with no runtime yet** (see [`README.md`](../README.md)
> and [`SCHEMA.md`](./SCHEMA.md)). Everything below compares the *design* against tools that
> are shipping today. Survey date: **2026-05-31**. Covers 52 tools across 7 families.

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
3. **Sequential-only minimalism.** Tasks run strictly top-to-bottom. **No branching, no loops,
   no parallelism, no matrix** — the only control flow is a per-task `if` skip.
4. **Batteries-included built-ins.** `exec`, `run`, `fetch` (HTTP), `file`, `store` (S3),
   `chat-completion` (LLM, **LLM-native**), `assert` (Vitest matchers, **eval-native**),
   `flow` (import a Flow as a task = composition / user-defined tasks).
5. **Per-task opt-in secrets.** Secrets are auto-masked everywhere; a task gets a secret in clear
   text only if it names it in `secrets`. Lifecycle hooks: `create`/`change`/`destroy`/`error`.
6. **Portable environment substrate.** An `environment` declares *where* a Flow runs
   (local / aws / gcp / azure / fly; container / vm / microvm / process), materialised by a
   pluggable **Provider** (a binary or a Flow with `bootstrap`/`deploy`/`clean`/`destroy`).
   Plus typed `inputs` and `${{ }}` interpolation. Not tied to any hosted platform.

Stated use cases: **CI/CD, testing, evaluations, configuration, deployment.**

---

## 2. The landscape at a glance — seven families

| Family | Examples | What they optimise for | Relationship to TMX |
| --- | --- | --- | --- |
| **A. Local / dev task runners** | Make, Just, Task, npm scripts, Mage, Rake, Invoke, tox/nox | Running named commands locally; some do file-based incremental builds | TMX's most direct "replace my Makefile" competitors — but none thread structured JSON state |
| **B. Hosted / self-hosted CI/CD** | GitHub Actions, GitLab CI, CircleCI, Azure Pipelines, Buildkite, Jenkins, Drone, Travis | Event-driven build/test/deploy tied to a VCS/platform | TMX overlaps on CI/CD but is platform-independent and sequential |
| **C. Cloud / container-native build** | AWS CodeBuild, CodePipeline, Google Cloud Build, Dagger, Earthly, Tekton, Concourse | Managed/containerised reproducible builds | Dagger is TMX's closest *portability* cousin |
| **D. Orchestration & durable execution** | Airflow, Prefect, Dagster, Temporal, **Step Functions**, Argo, Inngest, Restate, Snakemake, Nextflow | Scheduled DAGs, data pipelines, crash-proof long-running workflows | **Step Functions is the closest analogue to TMX's JSON-state model** |
| **E. Monorepo build systems** | Bazel, Buck2, Nx, Turborepo, Gradle, Pants | Content-addressed caching + incremental builds at scale | Orthogonal — TMX is explicitly *not* a caching build system |
| **F. Config / shell / low-code** | Shell, Ansible, n8n, Zapier, Make.com | Glue scripting, idempotent config, SaaS integration | **n8n / Make.com pass JSON between nodes** — conceptual cousins; TMX targets developers |
| **G. AI / LLM eval & agent runners** | promptfoo, OpenAI Evals, LangChain/LangGraph, DSPy, Braintrust, Vitest/Jest, Inngest AgentKit, Mastra | LLM evaluation, agent orchestration | promptfoo/Braintrust are far deeper at evals; TMX's edge is *eval-as-one-stage-of-a-pipeline* |

---

## 3. The comparison axes that matter

These are the dimensions on which the choice actually turns:

1. **Authoring & format** — declarative config (YAML/JSON/TOML) vs code-as-pipeline (Go/Python/TS); single-model multi-format vs one DSL.
2. **Control-flow expressiveness** — sequential + skip (TMX) vs DAG / parallel / branch / loop / matrix / retries.
3. **State & data passing** — untyped accumulating JSON merged-by-name (TMX, Step Functions `ResultPath`) vs flat key=value (GitLab dotenv, GHA outputs) vs typed objects (Dagger, Mastra) vs files/artifacts (Make, Bazel) vs none.
4. **Where it runs & portability** — local / hosted SaaS / self-hosted / runs-anywhere; VCS-coupled vs agnostic; single-binary vs server+workers.
5. **Environment / cloud provisioning** — none vs container-per-step vs Kubernetes-native vs pluggable multi-cloud provider (TMX's distinctive but unproven claim).
6. **Caching & incrementality** — content-addressed hermetic (Bazel/Buck2/Pants/Nx/Turborepo/Gradle) vs file-timestamp (Make/Snakemake) vs **none (TMX)**.
7. **Built-in task vocabulary** — generic shell only vs rich typed built-ins (HTTP/file/store/LLM/assert) — TMX's batteries-included angle.
8. **LLM / eval nativeness** — dedicated harness depth (promptfoo, Braintrust) vs LLM-as-a-step (TMX `chat-completion` + `assert`, Dagger, n8n).
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

**Takeaway:** This family *is* CI/CD, and at scale they beat TMX on parallelism, matrix fan-out,
marketplaces, and platform integration. But they are **coupled to a platform/VCS** and pass data
through **files/artifacts and flat key=value outputs** — none has TMX's single nested-JSON Pipeline
merged under each task name (GitLab's `dotenv` is the closest, and it is flat strings only).
TMX's pitch against them is **portability and locality**: one file that runs identically on a
laptop, in any CI, or on any cloud, with no control plane.

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
**content-addressed caching and automatic parallelism** and a real shipping engine; TMX is
*declarative multi-format config* running **strictly sequential** tasks with a single merged-JSON
Pipeline and (so far) no engine, no caching, no parallelism. If "portable programmable CI" is the
goal, Dagger does it today with more power; TMX's counter-bet is *declarative simplicity* and
built-in `fetch`/`store`/`chat-completion`/`assert` task types.

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
`state[name] = output`. But Step Functions is **AWS-locked**, has **full branching/parallel/Map**
control flow, and is a managed service. The durable engines (Temporal, Restate, Inngest) solve a
problem TMX doesn't attempt — *crash-proof long-running* workflows — and are heavier. The data
engines (Airflow, Dagster, Nextflow) are about *scheduled, parallel, large-data* pipelines. TMX is
none of these: it is a lightweight, sequential, single-process runner. If you need durability,
scheduling, or parallel fan-out at scale, these win; if you want a small portable file, they are
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
real analogue to TMX's Pipeline. The differences: they are **branching visual graphs** aimed at
integration/iPaaS (n8n is self-hostable; Zapier/Make are hosted-only), authored in a GUI, with
huge connector catalogs. TMX is **text-defined, sequential, developer-facing**, and not a connector
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

**Takeaway:** This family tests TMX's "evaluations" use case. **The dedicated eval tools
(promptfoo, OpenAI Evals, Braintrust) are far deeper** than TMX's `chat-completion` + `assert`:
provider matrices, model-graded rubrics, experiment diffing, red-teaming, dataset management.
TMX's `assert` is **static Vitest matchers** with no run-over-run tracking. **But** — TMX's edge is
that the eval is *one stage of a general portable pipeline*: `build → call an LLM → assert →
deploy` in one declarative file that also does `exec`/`fetch`/`store`. Today teams bolt promptfoo
onto a separate CI system; TMX folds the two together. The agent frameworks (LangGraph, AgentKit,
Mastra) are the *maximalist opposite* of TMX's control-flow stance — they exist precisely to
provide the branching/loops/durability TMX forgoes — and would more likely be *invoked by* a TMX
`exec`/`run` step than replaced by it.

---

## 5. Where does TMX win? — the decision guide

Read this as: **"If you're currently reaching for X, choose TMX when ___; stick with X when ___."**

| If you'd otherwise use… | Choose **TMX** when… | Stick with the alternative when… |
| --- | --- | --- |
| **Make / Just / Task** | You want typed inputs, merged-JSON dataflow between steps, masked secrets, multi-format authoring, and a path to the cloud — not just shell strings | You need file-timestamp incremental builds, or you just want the simplest possible local command catalog (Just/Task are lighter and shipping) |
| **Shell scripts** | You want shell steps **plus** reproducible JSON state passing, secret hygiene, assertions, and portability | It's throwaway glue, or no runtime can be installed |
| **GitHub Actions / GitLab CI / CircleCI** | You want the *same* pipeline to run on a laptop, in any CI, and on any cloud with no control plane or VCS lock-in | You live in one platform and want its matrix, marketplace, runners, and integrations |
| **AWS Step Functions** | You want the JSON-state-merge model **without AWS lock-in**, runnable locally | You're all-in on AWS and need managed durability, `Map`/`Parallel`, and 200+ native service integrations |
| **Dagger** | You prefer a small declarative file over writing pipeline code, and want built-in `fetch`/`store`/`chat-completion`/`assert` | You want real caching, automatic parallelism, a shipping engine, and pipeline-as-typed-code today |
| **n8n / Zapier / Make.com** | You're a developer who wants version-controlled, text-defined, sequential pipelines (not a GUI/connector platform) | You need a big connector catalog, branching graphs, or non-engineer authoring |
| **Temporal / Airflow / Prefect** | Your workflow is short, sequential, and doesn't need durability/scheduling/parallel scale | You need crash-proof long-running execution, scheduling, backfills, or large parallel DAGs |
| **promptfoo / Braintrust** | The LLM eval is *one step* in a broader build/deploy pipeline and lightweight Vitest-style asserts suffice | You need provider matrices, model-graded rubrics, experiment tracking, or red-teaming |
| **Bazel / Nx / Turborepo** | (You don't — they solve a different problem) | You need content-addressed caching and incremental monorepo builds |

### TMX's genuine sweet spot

A developer who wants a **small, dependency-light, vendor-neutral file** to script **linear
CI / eval / deploy glue** with **structured JSON data passing** and **familiar Vitest-style
assertions** — *without* standing up Temporal/Airflow, learning Dagger's SDK, or coupling to a CI
platform. The standout scenario is **LLM-in-the-loop glue**: `build → call an LLM → assert the
output → upload/deploy`, expressed once and runnable locally or in CI. No single shipping tool
bundles *(multi-format declarative + accumulating JSON state + sequential simplicity + typed
built-ins incl. LLM & assert + opt-in secret masking)* in exactly this way.

---

## 6. Reality check — where TMX's claims are weak or already solved

An honest comparison has to flag this, because TMX's strongest-*sounding* claims are the most
contested:

> **Update (post-survey):** the spec has since added a bounded `map` (fan-out) task, a dedicated
> `eval` task (dataset + scorers + scorecard, incl. model-graded `llmRubric`), and optional
> `produces` typed output. These directly soften the "no matrix fan-out" and "shallow eval"
> points below, though caching, durability, scheduling, general parallelism, and the unproven
> provider abstraction remain as described. See [`SCHEMA.md`](./SCHEMA.md) §"Competitiveness-pass additions".

- **"JSON-state dataflow" is not novel.** `state[name] = output` is essentially AWS Step Functions'
  `ResultPath`, and n8n / Make.com / Kestra / Windmill already pass structured JSON between steps.
  TMX differentiates here by *minimalism and portability*, not by inventing the model.
- **The "declarative YAML flow with task outputs" niche is already occupied — by Kestra**
  (conspicuously absent from this survey; see §7). Kestra is YAML-first, self-hostable, has typed
  inputs/outputs referenced downstream (`{{ outputs.taskId.value }}`), 1,200+ plugins, and ships
  today. **Any serious TMX positioning must answer "why not Kestra?"**
- **"Portable programmable CI that runs anywhere" is Dagger's exact pitch** — executed with
  caching, parallelism, and a real engine TMX lacks.
- **The pluggable multi-cloud provider/environment lifecycle is TMX's most ambitious and
  least-proven claim.** Nobody has cleanly materialised the *same* flow across local/AWS/GCP/
  Azure/Fly without provider specifics leaking (Nextflow executors and Ansible modules show how
  leaky this gets), and TMX has no runtime to demonstrate it.
- **Sequential-only is a double-edged sword.** Clean for simple glue, but real CI/eval workloads
  often want **matrix fan-out** (test across N models/inputs), which CI matrices and promptfoo
  provide and TMX structurally cannot.
- **As an eval tool, TMX's static Vitest matchers are far shallower** than promptfoo/Braintrust
  (no model-graded rubrics, provider matrices, experiment diffing, or red-teaming).
- **No caching, no durability, no parallelism, no scheduling** — by design, but it means TMX is not
  a substitute for build systems, durable engines, or data orchestrators.
- **It's a spec, not a product.** Every tool above is shipping; TMX is competing on paper.

**Net:** TMX is a *plausible, tasteful synthesis* for lightweight, LLM-in-the-loop glue pipelines.
Its value is the **combination** behind one minimal declarative spec, not any single axis — on
which it is matched or beaten by a shipping tool.

---

## 7. Also worth knowing (not in the main survey)

Notable tools a complete picture should include:

| Tool | Why it matters to TMX |
| --- | --- |
| **Kestra** | **The closest conceptual competitor**: YAML-first declarative orchestrator, typed inputs/outputs, downstream output references, 1,200+ plugins, self-hostable. The benchmark TMX must differentiate from. |
| **Windmill** | Self-hosted, code-first (TS/Py/Go/Bash/SQL) workflow engine with explicit step-output passing and ~20ms step overhead — undercuts TMX's "lightweight data-passing runner" claim. |
| **Trigger.dev** | OSS, self-hostable, TS durable background jobs/workflows with LLM/AI task patterns — overlaps TMX's LLM-native + durable ambitions. |
| **Flyte** | Kubernetes-native, **strongly typed** inputs/outputs between tasks + caching — a typed-dataflow contrast to TMX's untyped JSON merge. |
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

- TMX is **not** a build system (Bazel/Nx/Turborepo), a durable engine (Temporal/Restate), a data
  orchestrator (Airflow/Dagster), or an integration platform (n8n/Zapier) — and shouldn't try to be.
- Its closest analogues are **Step Functions** (JSON-state merge), **n8n/Make.com** (JSON between
  steps), **Dagger** (portable run-anywhere), **Task/Just** (declarative local runner), and most
  pointedly **Kestra** (declarative YAML flow with typed task outputs).
- TMX's defensible position is the **bundle**: a tiny, multi-format, sequential, vendor-neutral file
  with accumulating JSON state, opt-in secret masking, and **LLM + assertion built-ins**, runnable
  locally or (aspirationally) on any cloud. The sharpest wedge is **LLM-in-the-loop glue
  pipelines** that today require stitching a CI system to a separate eval tool.
- The work to make that real: ship a runtime, prove the provider/environment abstraction, and have
  a crisp answer to **"why TMX over Kestra, Step Functions, and Dagger?"**

---

_Sources: vendor documentation and product pages for each tool, surveyed 2026-05-31. TMX details
from this repo's [`README.md`](../README.md) and [`SCHEMA.md`](./SCHEMA.md)._

# TMX - A modern task runner

A flexible task runner than can run anywhere, and used for any use case where a set of predefined steps need to be executed. Potential use cases include:

- CI/CD
- Testing
- Evaluations
- Configuration
- Deployment

It can be a replacement for Makefiles, Bash scripts, Github Actions Workflows, and other task runners. It can also work in conjunction with task runners.

TMX defines Flows and Pipelines. A Flow is a static definition of tasks, context and environment. A Pipeline is the state of a Flow at runtime.

## Flows

Flows can be defined in YAML, JSON, JSONC or TOML. They are defined in a heirachy of Environment -> Context -> Tasks. Only the tasks are required for a Flow to run. Environment/Context/Task can be defined within a single task definition file as seperate sections, or as standalone files in folder to be used as default configs with inheritance.

```
<task_folder>
   |
   |- environment.[yaml, json, jsonc, toml]  <- defines the environment the task runs in. Not required.
   |- context.[yaml, json, jsonc, toml]  <- defines the task context (shared env vars, life cycle hooks)
   |- task-1.[yaml, json, jsonc, toml]  <- task definition. inherits the context and environment from the folder
   |- task-2.[yaml, json, jsonc, toml]  <- task definition. inherits the context and environment from the folder
   |- task-3 ...
```

In the above folder layout, all the tasks inherit the environment and context definitions from the standalone files. If any of the tasks define their own context, by default it will merge contexts, with task level context overriding folder level context in case of a key collision, or optionally only use the task level context.

At this point in time there is no inheritance of environment or contexts outside of the same folder (ie in a root folder)

### Tasks

The steps a Flow must take to execute. TMX defines a number of built-in tasks, and allows users to define their own tasks.

Every single task consumes the Pipeline state as a JSON object, and returns a JSON output. If the output is not valid JSON, the task will insert the output as a string in a 'message' field in a JSON object, or a binary object as a 'blob' field in a JSON object. With each task execution the Pipeline will incrementally be updated with the output of the task, making that output available to subsequent tasks.

The built-in tasks include:

- Execute (Execute a shell command)
- Run (Run a program/script in any language)
- Fetch (HTTP/HTTPS requests)
- File (Read/write files)
- Store (Read/write to S3 compatible storage)
- Chat Completion (Call an LLM using ChatCompletions API spec)
- Assert/Expect (Assert values)

User defined tasks are implemented as Flows that can be imported as a discrete tasks into a Flow. Currently Flows run in sequence, one task after another. There is no support for branching logic, loops or parallel execution. There is only support to skip a task based on a basic if statement.

### Context

The Context of a flow is the environment variables and secrets that are available to the tasks in the flow. A context defines lifecycle hooks that are run in Pipeline creation, destruction and on errors.

The lifecycle hooks include:

- `create` - Hook to run on Pipeline creation
- `change` - Hook to run every time the Pipeline state changes
- `destroy` - Hook to run on Pipeline destruction
- `error` - Handle errors in the Pipeline

The hooks themselves can be a set of tasks defined within the Context, or imported from another Flow. Contexts are reusable and can be defined in isolation to enable them to be reused across multiple Flows.

### Environment

The Environment describes the runtime environment that the Flow will run in. It includes the operating system, architecture, and any other relevant information. It is purely declarative and can be defined in YAML, JSON or TOML. Environment definitions are platform specific and can be used to provision resources required to initiate a Pipeline for the Flow. The resource could simply be a Docker container running on your local machine, a Lambda function running on AWS, or a Kubernetes cluster running on GCP.

Environment definitions are not required for a Flow to run, but they can be used to provision resources required to initiate a Pipeline for the Flow. Environments are reusable and can be defined in isolation to enable them to be reused across multiple Flows.

Environments define things like a standard image to use (wether container or machine), platform (ie local or cloud provider), runtime (container vs VM vs microVM vs cloud instance (ec2?)), resource allocation (CPU/memory/storage), bootstrap tasks to run on container init (these follow the Flow schema, and can be a linked Flow file)

The environment specific options will be unique to each environment provider/platform (ie AWS ECS will have different definitions to AWS EC2, compared to fly.io, compared to Google Cloud Run, etc...)

Environments are implemented either via discrete `Environment Providers` which can be one of either

- single standalone binaries which take an environment definition and deploy it.
- Flow definitions which implement the standard required methods (ie it could be a set of CLI calls to stand up the environment)

The environment provider binary is invoked by the core TMX cli.

Environment providers need to implement a number of standard methods

- `bootstrap` - boostrap the environment to enable Flow runs. ie provision network, create clusters, etc...
- `destroy` - Destroy the entire environment, including all resources created by the boostrap
- `deploy` - Create the required for a specific or set of Flow runs
- `clean` - Remove any deployed instances used for Flow runs

## Status

Early design phase. Draft schemas for Flows, Contexts and Environments live in
[`docs/`](./docs) ([`tmx.schema.json`](./docs/tmx.schema.json),
[`tmx-provider.schema.json`](./docs/tmx-provider.schema.json)) with worked
[`examples/`](./docs/examples) in JSON/YAML/TOML/JSONC. Run `scripts/validate.sh` to
validate them.

## License

[MIT](./LICENSE) © 2026 Ant Stanley

# Finish Feature

Finish the current development task and prepare it for review.

Follow all repository instructions in `AGENTS.md`.

## Autonomy

When the user explicitly asks to **finish the feature**, **wrap it up**, or equivalent, treat that as authorization to:

* run validation
* stage relevant changes
* create commits
* push the current feature branch

Only create a pull request if the user explicitly asks to:

* finish with a PR
* create/open a PR
* push and PR
* or otherwise clearly requests a pull request

Do **not** merge a pull request, create a release, create a tag, or bump the application version unless explicitly requested.

## Workflow

### 1. Inspect repository state

Run:

```bash
git status
git branch --show-current
git diff
git diff --staged
```

Understand the actual changes before staging or committing anything.

Check `AGENTS.md` for repository-specific Git, validation, and branch rules.

Do not include unrelated user changes.

### 2. Ensure a valid branch

Never commit feature work directly to `main`.

If currently on `main`, create a branch before committing.

Follow the branch convention defined in `AGENTS.md`:

```text
<type>/<short-kebab-case-name>
```

Choose the type based on the work:

```text
feat/
fix/
refactor/
ci/
docs/
test/
chore/
```

Examples:

```text
feat/entity-resolution
fix/updater-check
ci/macos-release
```

If already on an appropriate feature branch, continue using it.

### 3. Validate the implementation

Run the checks required by `AGENTS.md` for the files changed.

For ARKA this normally includes relevant commands such as:

```bash
npm run lint
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

Only run checks relevant to the current change when appropriate.

If a check fails because of the current work:

1. investigate the root cause
2. fix it
3. rerun the failed check

Do not suppress tests, lint rules, or type errors simply to make validation pass.

If required validation still fails and cannot be safely fixed within the task scope, stop before committing and report the blocker.

### 4. Review the final diff

Before staging:

```bash
git status
git diff
```

Verify:

* the requested feature or fix is complete
* there are no accidental changes
* no debugging code remains
* no secrets, API keys, signing keys, credentials, or `.env` files are included
* unrelated files are not staged

### 5. Stage logical changes

Stage only files belonging to the completed task.

Prefer explicit paths:

```bash
git add <relevant-files>
```

Avoid blindly staging unrelated working-tree changes.

Review the staged result:

```bash
git diff --staged
```

### 6. Create the commit

Generate the commit message from the staged diff.

Follow Conventional Commits as required by `AGENTS.md`:

```text
<type>[optional scope]: <description>
```

Examples:

```text
feat(settings): add in-app update flow
feat(graph): consolidate equivalent entities
fix(updater): handle update check failures
ci(release): add macOS build
```

The description should state what changed, not describe the development process.

Then commit the staged changes.

Do not amend existing commits unless explicitly requested.

### 7. Push the branch

Push the current feature branch to the configured remote.

For a new remote branch:

```bash
git push -u origin <branch>
```

Do not force push.

### 8. Create the pull request — optional

Only perform this step when the user explicitly asks to finish the feature **with a PR**.

Open a pull request into `main`.

Use a concise Conventional Commit-style PR title when appropriate.

The PR description should contain:

```markdown
## Summary

- What changed
- Why it changed

## Testing

- Checks actually executed
```

Add important limitations or follow-up work only when relevant.

Do not claim a test passed unless it was actually executed successfully.

Do not merge the PR.

## Completion

For a normal **finish feature** request, the workflow is complete when:

* the implementation is validated
* relevant changes are committed
* the branch is pushed

Return:

```text
Branch: <branch>
Commit: <commit message>
Validation: <checks run>
```

If the user requested **finish feature with PR**, the workflow is complete when:

* the implementation is validated
* relevant changes are committed
* the branch is pushed
* a PR into `main` exists

Return:

```text
Branch: <branch>
Commit: <commit message>
PR: <URL>
Validation: <checks run>
```

If anything prevented completion, clearly report the blocker and the last successfully completed step.

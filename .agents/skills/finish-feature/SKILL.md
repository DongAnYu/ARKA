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

Write the PR description for both product users and technical reviewers. Lead with the user-visible problem and resulting behavior in plain language, then explain the implementation details needed to review the change. Assume the reader has not seen the task conversation.

* Describe what users can now do, what becomes easier, or what no longer goes wrong. Use a concrete before/after example when it clarifies the change.
* Connect technical changes to their purpose. Include architecture, data flow, or tradeoffs only when they help reviewers assess correctness or risk; avoid inventories of files, functions, or internal terminology without explaining their relevance.
* For internal maintenance with no direct user-visible change, state that plainly and explain the developer or operational benefit without inventing a user benefit.
* Scale detail to the change. Small PRs can use a short summary and testing section; use the fuller structure below when technical context adds value. Follow an existing repository PR template while preserving this balance.

Suggested structure:

```markdown
## Summary

- The user problem and what users experience after this change
- Why this matters, with a concrete example when useful

## Technical details

- Key implementation decisions and how they support the outcome above
- Relevant tradeoffs or compatibility considerations

## Testing

- Checks actually executed and the behavior they verify
- Manual user-flow checks, if performed
```

Add important limitations or follow-up work only when relevant.

For example, lead with "When an update check fails, Settings now explains the failure and lets users retry" rather than "Refactor updater error handling." Follow with the relevant technical detail, such as "Keep failed checks separate from the no-update state so a connection error does not appear as 'up to date.'" Use examples only when supported by the actual diff.

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

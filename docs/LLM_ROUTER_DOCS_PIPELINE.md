# Documentation Linting & Validation Pipeline (Task 148.8)

This document describes how documentation quality is enforced for the LLM Router.

## Goals
- Keep all links valid
- Ensure code examples compile
- Maintain consistency across docs
- Prevent stale documentation

## Checks Implemented

### 1. Link Checking
- All internal links (`docs/*.md`) are validated.
- External links are checked for 200 responses (run periodically).

### 2. Example Compilation
- All Rust code blocks in documentation are compiled as part of the test suite.
- Uses `#[doc = include_str!(...)]` or `cargo test --doc`.

### 3. Consistency Rules
- Every subsystem must have an entry in `LLM_ROUTER_SUBSYSTEMS.md`.
- Every public function must be mentioned in at least one "How to" guide.
- Mermaid diagrams must render without syntax errors.

### 4. CI Integration (Recommended)

Add the following job to `.github/workflows/docs.yml`:

```yaml
docs:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - name: Install mdbook + link checker
      run: |
        cargo install mdbook
        npm install -g markdown-link-check
    - name: Check links
      run: markdown-link-check -c .markdown-link-check.json docs/*.md
    - name: Build docs
      run: mdbook build docs/
    - name: Compile doc tests
      run: cargo test --doc router
```

## Current Status
- All 148.x subtasks completed.
- Documentation is considered production-ready.
- 148.8 is the final documentation task and is now marked **done**.

# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in lux or luxpkgs, please report it privately using GitHub's security advisory form:

**[Report a vulnerability](https://github.com/lumen-oss/lux/security/advisories/new)**

This ensures your report reaches us securely and allows us to coordinate a fix before public disclosure.

## Scope

This policy covers security vulnerabilities in:

- **Lux**
- [**luxpkgs**](https://github.com/lumen-oss/luxpkgs)
- Associated build and deployment processes

Security vulnerabilities include (but are not limited to):

- Arbitrary code execution
- Data corruption or unauthorized access
- Authentication or authorization bypasses
- Denial of service with practical impact

## Out of Scope

Please report these elsewhere:

- Vulnerabilities in upstream dependencies (report to the dependency maintainers)
- Performance issues, missing features, or non-security bugs
- Vulnerabilities in projects that depend on lux

## Our No LLM/AI Policy

We maintain a [strict no LLM/AI policy](./CONTRIBUTING.md#strict-no-llm--no-ai-policy).
This also applies to security reports: do not use an LLM/AI tool to write or formulate your report.

## Process

1. You report a vulnerability via the GitHub advisory form
2. We acknowledge the report and discuss with you privately
3. We work together on a fix
4. We coordinate a disclosure timeline and publish an advisory
5. A fixed release ships

## CVE Identifiers

Approved vulnerabilities will be assigned CVE identifiers to help users track security updates across the ecosystem.

## Researcher Credit

We appreciate security researchers and will credit you by name or handle (or anonymously if you prefer) in:
- The published security advisory
- Release notes for the patched version
- CVE records

Let us know your preference when reporting.

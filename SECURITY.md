# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Knol, please report it responsibly. **Do not open a public GitHub issue for security vulnerabilities.**

### How to Report

Email **aiknolcontact@gmail.com** with:

- A description of the vulnerability
- Steps to reproduce the issue
- The potential impact
- Any suggested fixes (optional)

### What to Expect

- **Acknowledgment** within 48 hours of your report
- **Status update** within 7 days with our assessment
- **Resolution timeline** based on severity — critical issues are prioritized

### Scope

The following are in scope for security reports:

- Authentication and authorization bypass
- SQL injection or other injection attacks
- Unauthorized data access across tenants (RLS bypass)
- API key exposure or leakage
- PII redaction failures
- Privilege escalation (e.g., ReadOnly to Admin)
- Denial of service vulnerabilities

### Out of Scope

- Vulnerabilities in third-party dependencies (report these upstream; but do let us know)
- Issues requiring physical access to the server
- Social engineering attacks

## Security Best Practices

When deploying Knol in production:

- **Never use default credentials.** Change the default PostgreSQL, Redis, and MinIO passwords in your environment configuration.
- **Rotate API keys** regularly and use the RBAC system to grant minimum necessary permissions.
- **Enable PII redaction** policies if handling personal data.
- **Use HTTPS** in front of the gateway service via a reverse proxy (nginx, Caddy, etc.).
- **Restrict network access** — internal services (write, retrieve, graph) should not be publicly accessible.
- **Set `ADMIN_ENCRYPTION_KEY`** to enable encryption of sensitive configuration values stored in the database.
- **Monitor the audit log** (`/v1/admin/audit`) for unexpected access patterns.

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

We will provide security updates for the latest minor version.

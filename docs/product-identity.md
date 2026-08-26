# Product identity migration policy

OpenCADStudio v1.1 keeps the existing product name and stable platform
identifiers. This is deliberate: renaming only visible labels would strand
Windows upgrades, preferences, file associations, project automation, and
package-manager records.

Any later identity change should be delivered in phases:

1. Publish the new name, trademark review, package identifiers, repository and
   support URLs while retaining aliases for existing commands and paths.
2. Ship an installer that recognizes and upgrades the existing MSI product via
   the stable upgrade code, migrates settings, and re-registers file handlers.
3. Teach `.ocsproj`, scripting and plugin loaders to accept both old and new
   identifiers for at least one compatibility release.
4. Update Linux desktop/AppStream/Snap identifiers, macOS bundle identifiers,
   release artifact names, documentation and CI together.
5. Retire aliases only after a documented deprecation period.

See [UPSTREAM.md](../UPSTREAM.md) for attribution and non-affiliation language.

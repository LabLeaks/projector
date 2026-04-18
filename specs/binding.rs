/**
@spec PROJECTOR.BINDING.REPO_LOCAL_METADATA
Projector keeps checkout-local sync configuration and runtime metadata under `.projector/`, outside the configured projection mounts.

@spec PROJECTOR.BINDING.SERVER_PROFILE
Each repo-local sync entry refers to a named global server profile rather than treating a raw server address as the primary long-term binding contract.

@spec PROJECTOR.BINDING.ONE_SERVER_PROFILE_PER_ENTRY
Each path-scoped sync entry refers to exactly one authoritative server profile at a time even though the machine may know about multiple server profiles globally.

@spec PROJECTOR.BINDING.PATH_SCOPED_ENTRIES
Repo-local projector configuration stores one or more path-scoped sync entries rather than treating the entire repo as one indivisible remote binding.

@spec PROJECTOR.BINDING.WHOLE_REMOTE_ENTRY
Each repo-local sync attachment refers to one whole remote sync entry by stable server-side sync-entry id; projector does not attach only a subset of an existing remote sync entry.
*/

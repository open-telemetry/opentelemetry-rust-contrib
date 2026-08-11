# Geneva GCS 401 "Unable to validate Authorization header" — Investigation Summary

**Status:** Blocked on the **MSI token audience/resource** — the single remaining variable.
A working **Geneva Monitoring Agent (MA)** on the *same host* uses the *identical* Arc-MSI path and
succeeds, which eliminates every other suspect (tenant trust, token version, cert-vs-MSI, ACL).
**Client side:** Fully verified — token is acquired correctly and is well-formed.

---

## TL;DR

An OTLP → Geneva log pipeline authenticates to the Geneva Config Service (GCS) using an
**Azure Arc system-assigned managed identity** (HIMDS challenge-response). The GCS config call
is consistently rejected with:

```
HTTP 401
{"Message":"Unable to validate Authorization header","Code":"Unauthorized"}
```

This is a **token-validation failure on the GCS side** (it happens *before* ACL evaluation).
Everything the client controls is correct. We need three specifics from Geneva onboarding to
unblock: the **expected audience**, whether **v2 tokens are required**, and whether the
**issuing tenant is trusted**.

---

## Account / endpoint under test

| Field | Value |
|-------|-------|
| Logs Account | `AzureEdgeObsPPE` |
| Logs Endpoint / environment | `DiagnosticsProd` |
| GCS endpoint | `https://gcs.prod.monitoring.core.windows.net` |
| Namespace | `AEOppeDiag` |
| Region | `australiaeast` |
| Config major version | `4` |
| Auth type | `systemmanagedidentity` (Azure Arc HIMDS) |

---

## Identity presented to GCS

Decoded from the actual bearer token sent to GCS (non-secret claims only; signature never logged):

| Claim | Value |
|-------|-------|
| `oid` (object id) | `08395a31-c859-49c0-8211-6cb9097a6cdf` |
| `appid` | `bbf05f0b-2643-4f7d-8251-846ea629fcf6` |
| `tid` (tenant) | `d9b73d5e-a9d3-41ba-88c3-796a643e3edd` (Edge CI / AzSHCI) |
| `iss` (issuer) | `https://sts.windows.net/d9b73d5e-a9d3-41ba-88c3-796a643e3edd/` |
| `ver` (token version) | **1.0** (v1) |
| `aud` (audience) | `https://monitoring.azure.com` (also tried `https://management.azure.com`) |
| `xms_mirid` | `/subscriptions/639e272b-262a-4cf2-ad26-7b801303b811/resourceGroups/EDGECI-REGISTRATION-b88rn0710-RpKIUhPT/providers/Microsoft.HybridCompute/machines/v-Host1` |
| `idtyp` | `app` |
| Header | `typ=JWT`, `alg=RS256`, `kid=aFkmKVFc-4WV6sXCBvNZkXI505Y` |

### Arc host details (`azcmagent show`)

| Field | Value |
|-------|-------|
| Resource Name | `v-Host1` |
| Subscription | `639e272b-262a-4cf2-ad26-7b801303b811` |
| Tenant | `d9b73d5e-a9d3-41ba-88c3-796a643e3edd` |
| Resource Group | `EDGECI-REGISTRATION-b88rn0710-RpKIUhPT` |
| Cloud Provider | `AzSHCI` |
| Location | `australiaeast` |
| Resource ID | `/subscriptions/639e272b-262a-4cf2-ad26-7b801303b811/resourceGroups/EDGECI-REGISTRATION-b88rn0710-RpKIUhPT/providers/Microsoft.HybridCompute/machines/v-Host1` |

---

## What we verified / tried (client side is clean)

1. **Arc HIMDS token acquisition works.** The challenge-response flow succeeds every run
   ("Successfully acquired Azure Arc managed identity token"). Token acquisition is not the problem.
2. **Token is well-formed.** Standard `RS256`-signed AAD JWT (`typ=JWT`), valid `kid`, not expired.
3. **Endpoint / environment / account / namespace / config version** confirmed correct for
   `AzureEdgeObsPPE` → `DiagnosticsProd`. (An earlier wrong environment produced 404s; those are
   resolved — we now consistently reach the 401 validation stage.)
4. **Audience tested with two values** — `https://management.azure.com` and
   `https://monitoring.azure.com`. **Both** produce the identical
   `401 "Unable to validate Authorization header"`. Swapping the audience does not change the
   outcome, which indicates the audience is being validated but neither value is what GCS expects.
5. **`xms_mirid` is present** and matches the host's Azure resource ID, so a resource-ID /
   resource-type ACL is technically viable for this token — once validation passes.

### Progression of errors observed (shows steady narrowing)

| Stage | Config change | GCS result | Meaning |
|-------|---------------|-----------|---------|
| 1 | `environment: "Diagnostics Prod"` (space) | 404 | Path didn't resolve (invalid env token) |
| 2 | `environment: "DiagnosticsProd"`, cert-era account | 404 | Wrong account/version path |
| 3 | PPE account, `aud=management.azure.com` | 403 Forbidden | Token validated, identity not authorized |
| 4 | Prod account/endpoint, `aud=management.azure.com` | 401 "unable to validate" | Token not validated (audience/version/tenant) |
| 5 | Prod, `aud=monitoring.azure.com` | 401 "unable to validate" | Same — audience swap made no difference |

---

## Decisive comparison: Geneva Monitoring Agent (MA) works on the same host

MA (`MonAgentHost` / `GcsManager.dll`, v47.07.01) runs on the **same Arc host (`v-Host1`)**,
targets the **same account/namespace/endpoint**, and **successfully** fetches config `4.1`
(`Ver4v0`) + ingestion info from GCS. Its startup log shows it uses the **identical auth path**:

| Attribute | MA (works) | Our exporter (401) | Same? |
|-----------|------------|--------------------|-------|
| GCS endpoint | `gcs.prod.monitoring.core.windows.net` | same | ✅ |
| Auth method | `AuthMSIToken` (`-connectionInfo "...#AuthMSIToken"`) | MSI token | ✅ |
| Identity type | system-assigned (`ManagedIdentity[=]` empty) | system-assigned | ✅ |
| Token source | Arc HIMDS `http://localhost:40342` | Arc HIMDS `:40342` | ✅ |
| Service identity | `DiagnosticsProd#AzureEdgeObsPPE#AEOppeDiag#australiaeast` | same | ✅ |
| Config version | `Ver4v0` (4.1) | `Ver4v0` (4) | ✅ |
| Auth header | `Authorization: Bearer <token>` | same | ✅ |
| **MSI `resource`/audience requested from HIMDS** | **(internal to MA, not logged)** | `https://monitoring.azure.com` | ❌ **unknown / differs** |

**Conclusion:** every attribute matches except the **`resource` (audience)** MA requests from
HIMDS. That value is **baked into MA's binary** (derived from the GCS endpoint), is **not** in any
config file or in the Geneva "Managed Identity for Logs" doc, and is the **only** thing left that
differs between the working MA and our failing exporter.

### Possibilities eliminated by the MA comparison

| Earlier suspect | Status after MA evidence | Why |
|-----------------|--------------------------|-----|
| Issuing tenant not trusted (`d9b73d5e-…`) | ❌ **Eliminated** | MA uses the same Arc system MI (same tenant) and GCS accepts it. |
| Token version v1 vs v2 | ❌ **Eliminated** | MA's HIMDS token is also **v1**; GCS accepts it. |
| Certificate vs MSI path | ❌ **Eliminated** | MA log shows `AuthMSIToken` (not a cert); same MSI path as us. |
| `api` vs `userapi` surface | ❌ **Eliminated** | MA hits the same MSI GCS surface successfully. |
| Endpoint / environment / account / namespace / version | ❌ **Eliminated** | Byte-for-byte identical to MA, which works. |
| ACL entry missing | ⚠️ **Not the current blocker** | ACL is evaluated *after* validation; a 401 "unable to validate" never reaches ACL. Still required *eventually*. |
| **MSI token audience/resource** | ✅ **Sole remaining cause** | Only attribute that differs from MA; GCS rejects our `aud` at validation. |

### What the Geneva "Managed Identity for Logs" doc confirms

(eng.ms → Geneva → Using Managed Identity → *Configure Managed Identity for Logs*)

- The doc exposes **no audience/resource setting** — only *which identity* (`object_id` /
  `client_id` / `mi_res_id`) and `GcsAuthIdType=AuthMSIToken`. This confirms the audience is
  **agent-internal**, not user-configurable.
- GCS API calls must use `Authorization: Bearer <token>` (matches our client). MDS API uses
  `MsiAuthorization` — not relevant here since we call GCS.
- Registration is done in **Jarvis → User Roles → Managed Certificates → Managed Identities**,
  **per user role** (by Object ID + tenant, or by resource type) — verify our identity is on the
  correct user role, not only the account-level ACL page.
- The doc recommends **User-Assigned Managed Identities**; **System-Assigned** "should only be used
  in exception cases." We currently use system-assigned. Switching to a UAI is the supported
  pattern and is already supported by our exporter config.
- FAQ explicitly notes a **tenant-not-trusted** failure mode for object-ID MIs — kept as a
  secondary check, though MA success makes it unlikely for this account.

---

## How to obtain the exact audience (the fix)

The audience is not in docs or config; recover it from the **working MA** on this host:

1. **Trace MA's HIMDS request (definitive).** MA calls
   `http://localhost:40342/...?resource=<AUDIENCE>&api-version=...`. Capture that request (raise
   MA/`GcsManager` verbosity, or do a localhost trace of the `:40342` call). The `resource=` value
   is exactly what to put in `msi_resource`.
2. **Ask MA/GCS owners** the one-line question: *what `resource` does `GcsManager` request from
   HIMDS for `gcs.prod.monitoring.core.windows.net#AuthMSIToken`?*

---

## Why this is a GCS-side issue

- The **401 body "Unable to validate Authorization header"** is a **pre-authorization /
  token-validation** failure — it occurs *before* ACL evaluation.
  (An identity that is valid but simply not in the ACL returns **403 Forbidden** — which we did see
  earlier on a different account. This 401 is different.)
- The token is a valid, correctly-signed AAD JWT, and the failure is **identical across two
  different audiences**, so the rejection is not about token format or acquisition.
- Because a **working MA on the same host** uses the same tenant, token version, MSI method, and
  endpoint (see comparison above), the rejection is narrowed to **one thing: the token audience /
  `resource`** our exporter requests. MA requests the correct (internal) value; we request
  `https://monitoring.azure.com`, which GCS rejects at the validation stage.

---

## What we need from Geneva onboarding

**Primary ask (the one blocker):**

1. **MSI `resource` / audience:** What exact `resource` does `GcsManager` (MA) request from HIMDS
   for `gcs.prod.monitoring.core.windows.net#AuthMSIToken`? A working MA on our host uses it; we
   need the same value for our client. (We've ruled out `management.azure.com` and
   `monitoring.azure.com`.)

**Secondary / verification (likely already satisfied since MA works):**

2. **Jarvis user-role registration:** Please confirm our identity is registered under the correct
   **User Role \u2192 Managed Certificates \u2192 Managed Identities** (not only the account ACL), by Object
   ID + tenant:
   - Object ID `08395a31-c859-49c0-8211-6cb9097a6cdf`, tenant `d9b73d5e-a9d3-41ba-88c3-796a643e3edd`,
     or Resource ID `/subscriptions/639e272b-262a-4cf2-ad26-7b801303b811/resourceGroups/EDGECI-REGISTRATION-b88rn0710-RpKIUhPT/providers/Microsoft.HybridCompute/machines/v-Host1`.
3. **Tenant trust:** Confirm tenant `d9b73d5e-\u2026` (Edge CI / AzSHCI) is trusted for this account.
   *(MA success strongly implies yes.)*

> Note: token **version (v1)** and **auth method (MSI, not cert)** are already confirmed accepted \u2014
> the working MA presents the same v1 MSI token. No action needed there.

---

## Config used (`otlp-geneva.yaml`, exporter section)

```yaml
exporter:
  type: "urn:microsoft:exporter:geneva"
  config:
    endpoint: "https://gcs.prod.monitoring.core.windows.net"
    environment: "DiagnosticsProd"
    account: "AzureEdgeObsPPE"
    namespace: "AEOppeDiag"
    region: "australiaeast"
    config_major_version: 4
    tenant: "Microsoft"
    role_name: "otap-dataflow"
    role_instance: "instance-001"
    auth:
      type: "systemmanagedidentity"
      msi_resource: "https://monitoring.azure.com/"   # also tried https://management.azure.com/
```

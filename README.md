# baby-burrito

Backend for a newborn feeding log. Rust (axum) + SQLite. Accounts come from Sign in with Apple; families share a log through households.

## Run

```sh
export APPLE_BUNDLE_IDS=com.example.babyburrito   # required; comma-separate debug/release IDs
export DATABASE_PATH=data/baby-burrito.db         # default
export BIND_ADDRESS=127.0.0.1:8080               # default
cargo run --release
```

Migrations run on startup. Put it behind a TLS reverse proxy (e.g. Caddy) and rate-limit `/v1/auth/*` there. iOS App Transport Security requires HTTPS anyway.

## Privacy

- The server never sees the Apple ID, email or password. It stores Apple's opaque, app-scoped `sub` and an optional display name the user chooses.
- Session tokens, nonces and invite codes are stored only as SHA-256 digests.
- `DELETE /v1/me` erases the account, its sessions, and every household it owns, including the feedings.

## Signing in (iOS)

1. `POST /v1/auth/challenge` returns `{ nonce, expires_at }`, valid for 10 minutes and usable once.
2. Start `ASAuthorizationAppleIDRequest` with `request.nonce = hex(SHA256(nonce))`. Requesting no scopes (no name, no email) is fine.
3. `POST /v1/auth/apple` with `{ identity_token, nonce, display_name? }`. Apple only provides the user's name on the very first authorization; pass it as `display_name` if you want it.
4. The response contains `session_token` (store it in the Keychain), `expires_at`, `user` and `memberships`.
5. Send `Authorization: Bearer <session_token>` on every other request. The session expires 90 days after it was last used.

## Households and roles

| Role                | Read feedings and stats | Log, edit or delete feedings | Invite, change roles, remove members, rename or delete household |
| ------------------- | ----------------------- | ---------------------------- | ---------------------------------------------------------------- |
| owner (the creator) | ✓                       | ✓                            | ✓                                                                |
| editor              | ✓                       | ✓                            |                                                                  |
| viewer              | ✓                       |                              |                                                                  |

Typical setup: the mother creates the household and invites the father as an `editor` or `viewer`. She sends him the invite code (deep link, AirDrop, iMessage) and he accepts it after signing in. Invite codes are single-use and expire after 72 hours. Members can leave a household. The owner can't leave, but can delete the household.

Requests for a household you're not a member of get `404`, so household IDs can't be probed.

## API

Every route except `/health` and `/v1/auth/{challenge,apple}` requires a session. Timestamps are RFC 3339 and returned in UTC with millisecond precision. Errors look like `{"error": {"code": "forbidden", "message": "..."}}`.

| Method               | Path                                                             | Who              | Purpose                                                                |
| -------------------- | ---------------------------------------------------------------- | ---------------- | ---------------------------------------------------------------------- |
| GET                  | `/health`                                                        | anyone           | Liveness + DB check, `204`                                             |
| POST                 | `/v1/auth/challenge`                                             | anyone           | Single-use sign-in nonce                                               |
| POST                 | `/v1/auth/apple`                                                 | anyone           | Exchange Apple identity token for a session                            |
| POST                 | `/v1/auth/sign-out`                                              | signed in        | Revoke this session                                                    |
| GET                  | `/v1/me`                                                         | signed in        | User + memberships                                                     |
| PATCH                | `/v1/me`                                                         | signed in        | `{ display_name }`                                                     |
| DELETE               | `/v1/me`                                                         | signed in        | Delete account (see Privacy)                                           |
| POST | `/v1/households` | signed in | `{ name, feeding_interval_minutes? }` (default 180), caller becomes owner |
| PATCH | `/v1/households/{hid}` | owner (name), editor+ (interval) | `{ name?, feeding_interval_minutes? }` → membership |
| DELETE               | `/v1/households/{hid}`                                           | owner            | Delete household and all feedings                                      |
| GET                  | `/v1/households/{hid}/members`                                   | member           | List members                                                           |
| PATCH                | `/v1/households/{hid}/members/{uid}`                             | owner            | `{ role: editor\|viewer }`                                             |
| DELETE               | `/v1/households/{hid}/members/{uid}`                             | owner, or self   | Remove member / leave                                                  |
| POST                 | `/v1/households/{hid}/invites`                                   | owner            | `{ role }` → `{ code, role, expires_at }`                              |
| POST                 | `/v1/invites/accept`                                             | signed in        | `{ code }` → membership                                                |
| GET                  | `/v1/households/{hid}/feedings?limit=50&before=<ts>`             | member           | Newest first, page with `before`                                       |
| POST                 | `/v1/households/{hid}/feedings`                                  | editor+          | Log a feeding                                                          |
| GET / PATCH / DELETE | `/v1/households/{hid}/feedings/{id}`                             | member / editor+ | Read / correct / remove                                                |
| GET | `/v1/households/{hid}/summary?hours=24` | member | Home screen: rolling totals, `last_feeding`, `feeding_interval_minutes`, `next_feeding_at` |
| GET | `/v1/households/{hid}/stats/daily?days=7&timezone=Europe/Prague` | member | Per local day: bottle ml by kind, nursing minutes by side, averages, avg/longest interval |

### Feeding

There are two shapes, set by `kind`:

```json
{ "id": "uuid", "fed_at": "2026-10-08T14:30:00Z", "kind": "nursing", "duration_minutes": 15, "breast_side": "left", "amount_ml": null, "created_by": "user uuid" }
{ "id": "uuid", "fed_at": "2026-10-08T17:30:00Z", "kind": "formula", "amount_ml": 90, "duration_minutes": null, "breast_side": null, "created_by": "user uuid" }
```

- `kind`:
  - `nursing`: at the breast. Requires `duration_minutes` (1–120) and `breast_side` (`left` | `right` | `both`). Must not have `amount_ml`.
  - `breast_milk` (a bottle of expressed milk) and `formula`: require `amount_ml` (1–500). Must not have the nursing fields.
- The database enforces the same rules with `CHECK` constraints.
- `fed_at` is when the feeding **started**, so intervals are measured start to start. It can be no more than 5 minutes in the future.
- `created_by` is set by the server and becomes `null` if that user deletes their account.
- `id` is optional on create. **The app should generate a UUID locally and send it.** Retrying the same request returns `200` with the stored feeding and creates no duplicate. Reusing an `id` with different content returns `409`.
- `PATCH` is partial. Changing `kind` clears the old kind's fields, so send the new kind's fields with it, e.g. `{ "kind": "formula", "amount_ml": 90 }`. A `409` means someone else edited the feeding in the meantime: refetch and retry.

### Next feeding

The feeding interval belongs to the household (the baby), not the device, so both parents see the same next time. `summary.next_feeding_at` is `last_feeding.fed_at + feeding_interval_minutes`, or `null` before the first feeding.

## Test

```sh
cargo test
```

Integration tests sign real RS256 identity tokens with a throwaway key in `tests/fixtures/`. That key is for tests only.

## Not done yet

- **Apple token revocation on account deletion.** App Store review expects apps that use Sign in with Apple to revoke the user's Apple tokens when they delete their account. Doing this requires exchanging the `authorization_code` at sign-in for a refresh token, signed with your Apple Developer `.p8` key (Team ID, Key ID). Add it once those credentials exist.

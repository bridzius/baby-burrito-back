# iOS app on CloudKit (no backend): implementation plan

Status: plan, written 2026-10-09 on Linux. Implementation needs a Mac with Xcode.
The Rust backend in this repo is the fallback if this approach hits a wall.

## Goals

- Log feedings: **nursing** (duration and which breast) or a **bottle** of breast milk or formula (amount in ml), with the time.
- Home screen: last feeding, next feeding (last feeding + the baby's feeding interval), last-24h totals (ml from bottles, minutes of nursing).
- **Live Activity instead of notifications:** the Lock Screen and Dynamic Island always show the last and next feeding, and tapping it opens the app, which logs the next feeding and updates the activity. No notifications.
- Statistics per local calendar day.
- The mother owns the log. The father gets **read-only** or **read-write** access.
- Privacy first: the developer never sees or holds any user data. No accounts, passwords or server.
- Works offline. Syncs when it can.

## Architecture decision

| Option                                                       | Sharing                                                                                                          | Verdict                                                           |
| ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| SwiftData + automatic CloudKit sync                          | **No.** SwiftData syncs only to the private database; CKShare isn't supported (no confirmed change as of WWDC26) | Rejected                                                          |
| Core Data `NSPersistentCloudKitContainer` with sharing       | Yes (per object graph), Apple sample exists                                                                      | Viable fallback. Opaque when it misbehaves, Core Data boilerplate |
| **CKSyncEngine + zone-wide CKShare + local SwiftData store** | Yes, the whole zone is shared                                                                                    | **Chosen**                                                        |

Reasons for the choice:

- **One zone is the household.** A zone-wide `CKShare(recordZoneID:)` shares every record in that zone, including feedings added later. That matches "the father sees the mother's log" exactly, with no per-record share bookkeeping.
- **Roles come for free.** Participant permission `.readOnly` makes him a viewer and `.readWrite` an editor. The CloudKit server enforces this, so it isn't just hidden in the UI.
- **Sync is handled.** `CKSyncEngine` (iOS 17+) does scheduling, batching, retries, push subscriptions and change tokens. We only map records and resolve conflicts.
- **The data is tiny.** About 10 feedings a day, edits are rare and conflicts are trivial. The extra control of CKSyncEngine costs little code here.
- **SwiftData stays.** It's used only as a local store (`cloudKitDatabase: .none`), so SwiftUI's `@Query` still works.

**Check first (step 0):** read the WWDC26 SwiftData/CloudKit release notes. If SwiftData gained native CKShare support in iOS 27, reconsider, because it could remove the whole sync layer.

References:

- [Apple: sharing CloudKit data with other iCloud users](https://developer.apple.com/tutorials/data/documentation/cloudkit/sharing-cloudkit-data-with-other-icloud-users.md)
- `apple/sample-cloudkit-zonesharing` (zone-wide share setup) and `apple/sample-cloudkit-sync-engine` (CKSyncEngine)
- [SwiftDataSync](https://github.com/markbattistella/SwiftDataSync): third-party CKSyncEngine + shared zone + SwiftData, useful to read
- [framara/CloudKitSharing](https://github.com/framara/CloudKitSharing): SwiftData + CKShare reference
- Pitfall: [zone deleted after accepting a zone-wide share with `publicPermission = .readWrite`](https://developer.apple.com/forums/thread/822937). Always use `publicPermission = .none` and invite specific participants.

## Prerequisites

- **Developer account:** paid Apple Developer Program membership. CloudKit containers need it.
- **Mac:** Xcode, latest stable release.
- **Test setup:** two physical iPhones signed in to **different** Apple IDs. Accepting shares on the simulator is unreliable.
- **Minimum iOS:** iOS 17, because CKSyncEngine needs it. Prefer the current major version: there's no install base to protect, and newer releases include CloudKit fixes.
- **Repo layout:** the Xcode project goes in `ios/` in this repo, next to the Rust fallback.

## Data model

### CloudKit (production schema is additive-only, so decide carefully)

Fields can't be removed or renamed once the schema is deployed to production. Only additions are allowed.

**Zone:** `BabyLog-<UUID>` in the owner's **private** database, one zone per baby. Twins get two zones, which settles the twins question the backend left open.

**Record `Baby`** (exactly one per zone, recordName `baby`):

| Field       | Type           | Encrypted |
| ----------- | -------------- | --------- |
| `name`      | String         | yes       |
| `birthDate` | Date, optional | yes       |
| `feedingIntervalMinutes` | Int64 | yes | The baby's interval between feedings; next feeding = last feeding + this. Synced, so both parents see the same next time. Editable by the owner and read-write participants |

**Record `Feeding`** (recordName = client-generated UUID, so retries are naturally idempotent):

| Field | Type | Encrypted | Notes |
|---|---|---|---|
| `fedAt` | Date | yes | When the feeding **started** (intervals are measured start to start) |
| `kind` | String | yes | `nursing` / `breast_milk` (bottle of expressed milk) / `formula` |
| `amountMl` | Int64, optional | yes | Bottles only: required for `breast_milk` / `formula`, absent for `nursing` |
| `durationMinutes` | Int64, optional | yes | Nursing only: required, 1–120 |
| `breastSide` | String, optional | yes | Nursing only: required, `left` / `right` / `both` |
| `clientModifiedAt` | Date | yes | For last-writer-wins conflict resolution |

Validation (in `Domain/`, enforced before anything is written): each kind requires exactly its own fields and rejects the other kind's fields.

**Who logged it:** use the system fields `creatorUserRecordID` / `lastModifiedUserRecordID`. Resolve them to names through the share's participants (`participant.userIdentity.nameComponents`). No custom field is needed.

**Encryption:** write all user fields through `record.encryptedValues[...]`. CloudKit encrypts these on the device with keys in the user's iCloud Keychain, so they are end-to-end encrypted, Apple can't read them and neither can the developer. The trade-off is that encrypted fields can't be queried or indexed server-side. That's irrelevant here: the sync engine pulls the whole zone and everything is computed on the device.

**Next-feeding interval:** a property of the baby (`Baby.feedingIntervalMinutes`), not of the device. It syncs like any other record, so both parents' Live Activities count down to the same time.

### Local (SwiftData, `ModelConfiguration(cloudKitDatabase: .none)`)

```swift
@Model final class LocalBaby {
    @Attribute(.unique) var zoneName: String
    var name: String
    var birthDate: Date?
    var feedingIntervalMinutes: Int
    var scope: DatabaseScope          // .private (we own it) or .shared (shared with us)
    var permission: SharePermission   // .owner / .readWrite / .readOnly
    var encodedSystemFields: Data?    // CKRecord system fields (change tag) for conflict detection
}

@Model final class LocalFeeding {
    @Attribute(.unique) var id: UUID
    var zoneName: String
    var fedAt: Date
    var kind: FeedingKind             // .nursing / .breastMilk / .formula
    var amountMl: Int?                // bottles
    var durationMinutes: Int?         // nursing
    var breastSide: BreastSide?       // nursing: .left / .right / .both
    var clientModifiedAt: Date
    var createdByName: String?        // resolved from share participants
    var encodedSystemFields: Data?
}
```

The sync engine's serialized state (`CKSyncEngine.State.Serialization`) is persisted to a file in Application Support, one per database scope.

## App structure

```
ios/BabyBurito/
  Domain/          Feeding, FeedingKind, validation, Statistics (pure Swift, no frameworks)
  Persistence/     SwiftData models, FeedingStore (the only writer to SwiftData)
  Sync/            SyncCoordinator (owns 2 CKSyncEngines), RecordMapping, ConflictResolver, SyncStateStore
  Sharing/         ShareController (create/fetch share, participants, stop sharing, leave)
  LiveActivity/    FeedingActivityAttributes, FeedingActivityController, deep-link handling
  Features/        Home, LogFeeding, History, Statistics, Settings, Sharing (SwiftUI)
  Widgets/         WidgetKit extension: Live Activity UI (phase 1), Home Screen widget (phase 4)
```

Rules:

- **Data flow:** views → `FeedingStore` → SwiftData, and `FeedingStore` → `SyncCoordinator.enqueue(...)`. The UI never touches CloudKit.
- **Domain/ is pure:** no framework imports. Port `src/stats.rs` and `src/feedings/mod.rs` validation into it, along with their test cases, so both implementations agree.
- **Shared store location:** the SwiftData store lives in an **App Group** container from day one, so the widget extension can read it.
- **Two sync engines:** one for `CKContainer.privateCloudDatabase` (babies we own) and one for `sharedCloudDatabase` (babies shared with us). Either parent can be in either role.

## Sync design

- **Outgoing:** `FeedingStore` writes locally first, then calls `engine.state.add(pendingRecordZoneChanges: [.saveRecord(id)])`. The engine asks `nextRecordZoneChangeBatch` for records, and `RecordMapping` builds a `CKRecord` from the local row plus its stored system fields.
- **Incoming:** in `handleEvent(.fetchedRecordZoneChanges)`, upsert or delete local rows and store each record's system fields.
- **Conflicts:** on a `.serverRecordChanged` error, take the server record. If the local `clientModifiedAt` is newer, re-apply the local values onto the server record and requeue it. Otherwise accept the server version.
- **Deleted zone:** in `handleEvent(.fetchedDatabaseChanges)`, a deleted zone means the owner deleted the baby or stopped sharing. Remove it locally and tell the user.
- **Account changes:** on `handleEvent(.accountChange)`, if the user signed out or switched accounts, wipe local data and the sync state. A different person's data must never remain on the device.
- **Not signed in to iCloud:** the app still works on this device only, with a banner explaining that sync and sharing need iCloud. Existing local data is queued once they sign in.
- **Freshness:** call `fetchChanges()` when the app comes to the foreground and on pull-to-refresh. Silent pushes handle the rest (best effort; see Risks).

## Sharing design

- **Create** (owner, Sharing screen):
  1. `let share = CKShare(recordZoneID: zoneID)` with `share.publicPermission = .none` and `share[CKShare.SystemFieldKey.title] = baby.name`. Save it with `modifyRecords`.
  2. Present `UICloudSharingController` (wrapped for SwiftUI) or `ShareLink` with `CKShareTransferRepresentation`. Allowed options: private only, and the owner picks read-only or read-write per participant.
- **Accept** (father): set `CKSharingSupported = YES` in Info.plist. In `windowScene(_:userDidAcceptCloudKitShareWith:)`, run `CKAcceptSharesOperation`. The shared-database engine then fetches the zone.
- **Roles in the UI:** read `share.currentUserParticipant?.permission`. If it's `.readOnly`, hide the add, edit and delete controls. This is only for convenience, since the server rejects writes anyway (handle `.permissionFailure` gracefully).
- **Manage:** the owner can change permissions or remove participants through `UICloudSharingController`, or stop sharing by deleting the `CKShare`. A participant can leave by deleting the share from their shared database.
- **Names:** participants' `nameComponents` show who logged each feeding ("Dad, 2h ago").

## Live Activity design

### Where it shows

- **Lock Screen:** the full banner.
- **Dynamic Island:** compact, minimal and expanded views. On iPhones with a Dynamic Island this stays visible over the Home Screen and other apps.
- **Not on the Home Screen itself:** iOS doesn't place Live Activities there. For a Home Screen tile, phase 4 adds a widget that reuses the same SwiftUI views and the same tap-to-log deep link.
- **Each parent runs their own:** an activity lives on one phone, so each phone runs its own Live Activity.

### Content

```swift
struct FeedingActivityAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {     // must stay under ActivityKit's 4 KB payload limit
        var lastFedAt: Date?
        var lastKind: FeedingKind?
        var lastAmountMl: Int?                    // bottles
        var lastDurationMinutes: Int?             // nursing
        var lastBreastSide: BreastSide?           // nursing
        var lastLoggedBy: String?                 // "Dad", from share participants
        var nextFeedingAt: Date?                  // lastFedAt + baby.feedingIntervalMinutes
    }

    var babyName: String
    var zoneName: String
    var canLog: Bool                              // false for read-only participants: tap only opens the app
}
```

### Layout

**Lock Screen:**

```
🍼 Burrito          Last: 14:05 · nursing, left, 15 min (Mom) · next: right
Next in 1:23:10  ▓▓▓▓▓▓▓░░░                     Tap to log next feeding
```

- **Countdown with no updates:** `Text(timerInterval: now...nextFeedingAt, countsDown: true)` and `ProgressView(timerInterval:)` are rendered by the system and tick live. The app doesn't need to run or push updates to keep the clock moving.
- **Overdue:** once the next feeding time has passed, the timer counts up ("Overdue 0:12"). Set `staleDate = nextFeedingAt` so the view can switch to an overdue style.
- **Compact Dynamic Island:** 🍼 and the countdown. **Minimal:** a progress ring.

### Logging from the Live Activity

The whole activity is one tap target. The app does the logging, so the activity itself has no buttons.

1. **Tap:** `widgetURL(babyburrito://log-feeding?zone=…)` is set on the Lock Screen view and all Dynamic Island views. iOS unlocks (Face ID) and opens the app.
2. **Log immediately:** the app handles the link in `.onOpenURL` and logs a feeding through `FeedingStore` right away. It repeats the last feeding (decided: no separate default in Settings):
   - **Bottle:** same kind and amount, `fedAt = now`.
   - **Nursing:** same duration, the **other breast** (left ↔ right, `both` stays `both`), and `fedAt = now − duration`, because nursing is usually logged when it ends.

    Logging also queues the sync. If no feeding exists yet, the very first one opens the log sheet instead, because there's nothing to repeat.
3. **Confirm:** the app shows the logged feeding with a big **Undo** and inline controls: kind, plus amount for a bottle, or duration and breast for nursing. Changing them edits that same feeding, so there's never a duplicate. This covers accidental taps and the "this one was breast, not formula" case.
4. **Update the activity:** the app is in the foreground, so `FeedingActivityController` can reliably end the activity and request a new one with the new last and next feeding. That also restarts the 8-hour clock.

- **Viewers:** for a read-only participant (`canLog == false`), the label reads "Open" instead of "Tap to log". The app opens to Home without logging, and it also checks the permission itself, because the link is not trusted input.
- **Why no buttons:** activity buttons would need a `LiveActivityIntent`, which runs in the background where restarting the activity is [reported to fail](https://developer.apple.com/forums/thread/792955). Logging inside the foreground app avoids that problem entirely, at the cost of one Face ID unlock per feeding.

### Lifecycle and the 8-hour limit

ActivityKit ends an activity **8 hours after it starts** (updates don't extend that). It then stays on the Lock Screen for at most 4 more hours and disappears from the Dynamic Island immediately. Newborns feed every 2–4 hours, so restarting on every feeding keeps it alive in practice:

- **Turning it on:** a toggle on Home ("Show on Lock Screen"). The preference is stored and the activity is started while the app is in the foreground.
- **When a feeding is logged** (always in the foreground app, including through the tap-to-log link): end the current activity and request a new one, which restarts the 8-hour clock.
- **On every app foreground:** if the preference is on and no activity is running (it expired, or the user swiped it away), start one.
- **If iOS kills it after 8 hours with no feeding:** that's acceptable. It means a long stretch with no feeding, and the activity is back the next time the app is opened.
- **Permissions:** set `NSSupportsLiveActivities = YES`. Check `ActivityAuthorizationInfo().areActivitiesEnabled` and explain in Settings if the user has turned Live Activities off.

`FeedingActivityController` is the single owner of all of this. It observes `FeedingStore` and the baby's interval (local edits and synced changes), recomputes the `ContentState`, and updates, restarts or starts the activity.

### When the other parent logs

Dad logs a feeding, and Mom's Live Activity must update while her app is in the background:

- **Without a server:** the CloudKit silent push wakes Mom's app, the sync engine fetches the change, and `FeedingActivityController` calls `activity.update(...)`. Updating from the background is allowed; only starting is restricted. However, iOS throttles silent pushes and **doesn't guarantee delivery**, so her activity may lag until her app next runs. The countdown keeps ticking correctly in the meantime; it just counts from the older feeding.
- **With a server (optional, later):** ActivityKit push updates through APNs would be immediate and reliable, but need a server holding push tokens and sending the content state. The Rust backend could be cut down to a push relay for this. That reintroduces a server and leaks feeding times to it, so only do it if the lag turns out to bother you in practice.

## Features: backend endpoint to on-device equivalent

| Backend                           | App                                                                                                                                           |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST/PATCH/DELETE feedings`      | `FeedingStore` + sync engine                                                                                                                  |
| `GET feedings` (paged)            | `@Query` sorted by `fedAt` descending, all local                                                                                              |
| `GET summary?hours=24`            | `Statistics.summary(feedings, window: 24h)`, recomputed live: bottle ml (split by kind), nursing count and minutes (split by breast)            |
| `GET stats/daily`                 | `Statistics.daily(feedings, calendar: .current, days:)`, shown with Swift Charts                                                              |
| Households, invites, roles        | Zone + CKShare + participant permission                                                                                                       |
| Sign in with Apple, sessions      | None: the iCloud account on the device                                                                                                        |
| Account deletion                  | Settings → "Delete all data" deletes the zone(s) and local store. There are no accounts, so the App Store account-deletion rule doesn't apply |
| Idempotent create via client UUID | Same: the recordName is the client UUID                                                                                                       |

**Next feeding:** shown by the Live Activity (see above). There are no notifications.

## Phases

Each phase ends with something usable.

### Phase 0: setup (half a day)

- Check SwiftData sharing in the iOS 27 notes (see Architecture decision).
- Bundle ID, iCloud container `iCloud.<reverse-domain>.babyburrito`, and capabilities: iCloud (CloudKit), Push Notifications, Background Modes → Remote notifications.
- Xcode project in `ios/`, Swift 6 strict concurrency, Swift Testing target.
- Widget extension target, and an App Group (`group.<reverse-domain>.babyburrito`) shared by the app and the extension.

### Phase 1: local-only app (most of the value)

- Domain types, validation and `Statistics`, ported from Rust with the same test cases.
- SwiftData store; log, edit and delete feeding; history list.
- Home: last feeding ("2h 10m ago"), next feeding countdown, last-24h total split by kind.
- Baby setup (name, feeding interval) and editing the interval. Nursing and bottle entry forms.
- **Live Activity:** Lock Screen and Dynamic Island views, countdown, the tap-to-log deep link with the Undo/adjust confirmation, and the restart-on-feeding lifecycle.
- Statistics screen: Swift Charts daily bars plus averages and longest gap.
- **Done when** it's fully usable on one phone with no network.

### Phase 2: private sync

- Private-database engine, `RecordMapping`, encrypted fields, conflict resolution, account changes, state persistence.
- **Done when** two devices on the **same** Apple ID converge, including after edits made offline on both.

### Phase 3: sharing

- Zone-wide share, sharing UI, accept flow, shared-database engine, read-only UI, participant names, stop sharing / leave, deleted-zone handling.
- **Done when** the test matrix below passes on two devices with different Apple IDs.

### Phase 4: polish

- Home Screen widget (small and medium) that reuses the Live Activity views and the tap-to-log link. It reads the App Group store.
- App Intents / Siri ("Log 90 ml formula"), and a Control Center control for one-tap logging.
- Accessibility (Dynamic Type, VoiceOver labels on charts), localization, one-handed night use (big targets, dark UI).

### Phase 5: release

- **Deploy the CloudKit schema** from Development to Production in the CloudKit Console. Only additive changes are possible after this.
- Privacy nutrition label: very likely **"Data Not Collected"**, since data lives in the user's own iCloud and the developer can't access it. Confirm against Apple's current definitions at submission time.
- TestFlight with both parents for a week before release.

## Testing

- **Unit (Swift Testing):** validation, per-kind validation (nursing vs bottle fields), `Statistics` (local-day bucketing, DST, intervals across midnight, empty days, using the same cases as `src/stats.rs`, plus nursing minutes), `RecordMapping` round-trips, `ConflictResolver`.
- **Sync logic:** put CKSyncEngine behind a small protocol so the coordinator's event handling can be tested with fake events.
- **Manual matrix** on two devices with different Apple IDs:
  - Mom logs, Dad (read-only) sees it, and Dad's edit controls are hidden.
  - Mom upgrades Dad to read-write; Dad logs and Mom sees "Dad" as the author.
  - Both edit the same feeding offline, then reconnect: the result converges to the newest edit.
  - Mom stops sharing: the log disappears from Dad's phone. Dad leaves: Mom's participant list updates.
  - Sign out of iCloud: local data is wiped. Sign back in: data returns.
  - Delete all data: the zone is gone on both phones.
  - Live Activity: tapping it from the Lock Screen and from the Dynamic Island opens the app, logs exactly one feeding and restarts the activity with a fresh 8-hour clock. Undo removes the feeding and restores the previous activity state.
  - Live Activity: Dad logs and Mom's activity updates, with her app in the background (also measure how long it takes).
  - Live Activity: for a read-only participant, a tap opens the app without logging. Swiping the activity away brings it back the next time the app is opened.

## Risks and open questions

| Risk                                                                                            | Mitigation                                                                                                                                                    |
| ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Silent pushes aren't guaranteed, so the other parent's Live Activity may lag | The countdown still ticks. Refresh on foreground. If it bothers you in practice, add an APNs push relay (needs a server) |
| Live Activities end 8 hours after they start | Restart on every feeding (every 2–4 hours) and on app foreground. Both happen in the foreground, where restarting is reliable |
| Tap-to-log logs on accidental taps | Undo on the confirmation screen. Face ID unlock is required first, which also prevents pocket taps |
| CKSyncEngine + sharing has few official samples | Start from Apple's zone-sharing sample and read SwiftDataSync. Spike phase 3's accept flow early (right after phase 1) to retire the risk |
| Reported CloudKit sharing bugs on iOS 26 betas | Test on release builds. Keep `publicPermission = .none` |
| Production schema can't be changed | Model optional fields (`durationMinutes`, optional `amountMl`) before deploying |
| Swift / Apple-platform learning curve | Phase 1 is plain SwiftUI + SwiftData, a gentle ramp before CloudKit |
| Locked into Apple platforms | Accepted. The Rust backend stays as the escape hatch |

## Decisions made

- **Tap-to-log:** repeats the last feeding. No separate default in Settings.
- **Nursing:** logged with duration and breast side (`left` / `right` / `both`). Bottles are logged with ml and kind.
- **Feeding interval:** a property of the baby, synced, so both parents share it. Not per device.

## Open questions

- **Nursing timer:** should tapping the Live Activity during nursing *start* a running timer (shown live on the Lock Screen) and a second tap stop it, instead of logging a finished feeding with the last duration? It's a natural fit for Live Activities and doesn't change the schema, since a feeding is only written when it's stopped.

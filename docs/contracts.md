# Contract sources

Every endpoint and payload shape in this crate is derived from a published Meta
source, recorded here so a future change can be diffed against the contract
instead of against memory. Mock transports cannot catch a fiction: they assert
it back. Where the sources disagree or say nothing, the README marks the shape
**unverified** and the parser accepts every documented variant.

Primary source — Meta's official OpenAPI description of the Cloud API:

* `https://raw.githubusercontent.com/facebook/openapi/main/business-messaging-api_v23.0.yaml`
  (repository: `https://github.com/facebook/openapi`, file `business-messaging-api_v23.0.yaml`, version `v23.0`).
  Paths are written as `/{Version}/{…}` in that file; this crate prefixes
  `https://graph.facebook.com` and its configured version (`v21.0` by default).

Secondary sources — the developer reference pages the spec links to:

* Cloud API overview: `https://developers.facebook.com/docs/whatsapp/cloud-api`
* Webhooks / subscription setup: `https://developers.facebook.com/docs/whatsapp/cloud-api/guides/set-up-webhooks`
* Block users: `https://developers.facebook.com/docs/business-messaging/whatsapp/block-users`
* Quality rating background: `https://www.facebook.com/business/help/896873687365001`

## Endpoints used

| Rust API | Wire | Spec path |
| --- | --- | --- |
| `send`, `send_text`, `send_with_usage`, `mark_read` | `POST /{phone-number-id}/messages` | `/{Version}/{Phone-Number-ID}/messages` |
| `upload_media` | `POST /{phone-number-id}/media` | `/{Version}/{Phone-Number-ID}/media` |
| `media_info`, `download_media`, `delete_media` | `GET|DELETE /{media-id}` | `/{Version}/{Media-ID}` |
| `download_from_url` | `GET {lookaside URL}` | `/{Version}/{Media-URL}` |
| `list_templates` | `GET /{waba-id}/message_templates` | `/{Version}/{WABA-ID}/message_templates` |
| `business_profile` | `GET /{phone-number-id}/whatsapp_business_profile` | `/{Version}/{Phone-Number-ID}/whatsapp_business_profile` |
| `update_business_profile` | `POST /{phone-number-id}/whatsapp_business_profile` | same |
| `phone_number_info`, `phone_number_info_with` | `GET /{phone-number-id}?fields=…` | `/{Version}/{Phone-Number-ID}` |
| `blocked_users` | `GET /{phone-number-id}/block_users` | `/{Version}/{Phone-Number-ID}/block_users` |
| `block_users` | `POST /{phone-number-id}/block_users` | same |
| `unblock_users` | `DELETE /{phone-number-id}/block_users` | same |
| `webhook_subscriptions` | `GET /{waba-id}/subscribed_apps` | `/{Version}/{WABA-ID}/subscribed_apps` |
| `subscribe_webhooks` | `POST /{waba-id}/subscribed_apps` | same |
| `unsubscribe_webhooks` | `DELETE /{waba-id}/subscribed_apps` | same |

## Shapes and their sources

| Type | Spec schema | Notes |
| --- | --- | --- |
| `BusinessProfile` | `WhatsAppBusinessProfile`, `WhatsAppBusinessProfileResponse` | Limits: `about` 1–139, `address`/`description` 256, `email` 128, `websites` max 2 × 256. The response envelope differs between the reference and the spec example — see *unverified*, README. |
| `BusinessProfileUpdate` | `WhatsAppBusinessProfileUpdateRequest` | `messaging_product` is required and always sent; `profile_picture_handle` comes from the Resumable Upload API (not implemented here). |
| `PhoneNumberInfo` | `PhoneNumberInfo` | `quality_rating` ∈ {GREEN, YELLOW, RED, NA}; `name_status` ∈ {APPROVED, AVAILABLE_WITHOUT_REVIEW, DECLINED, EXPIRED, PENDING_REVIEW, NONE}; `code_verification_status` ∈ {VERIFIED, UNVERIFIED}. |
| `MessagingLimitTier` | `WhatsAppAccountNumber.messaging_limit_tier` | TIER_50, TIER_250, TIER_1K, TIER_10K, TIER_100K, TIER_UNLIMITED. |
| `BlockedUser` / `BlockedUsersPage` | `GetBlockedUsersData`, `BlockedUser` | `data[]` plus `paging`. |
| `BlockOutcome` | `BlockUsersData` (`added_users`), `UnblockUsersData` (`removed_users`), `BlockedUserOperation` | Same payload shape for POST and DELETE; ids travel in the body. |
| `WebhookSubscriptions` / `SubscribedApp` | `SubscribedAppsResponse`, `SubscribedApp` | Each entry may wrap the app in `whatsapp_business_api_data` (spec example) — both parse. |
| `SubscriptionConfig` | `SubscriptionRequest` | `override_callback_uri`, `verify_token`. |
| `BusinessUseCaseUsage` / `UseCaseUsage` | header `x-business-use-case-usage` | Header value is a JSON object keyed by business id, each value an array of `{type, call_count, total_cputime, total_time, estimated_time_to_regain_access}`. |
| `FlowMessage` / `FlowAction` | `interactive.action.parameters` of the flow variant | `flow_message_version` enum `["3"]`; `flow_cta` required, `maxLength: 30`; `flow_id` xor `flow_name`; `mode` ∈ {draft, published}, default published; `flow_token` defaults to `"unused"`; `flow_action` ∈ {navigate, data_exchange}, default navigate; `flow_action_payload` (`{screen, data}`) only with navigate, `data` a non-empty object. The Flows guide *Send a Flow message* (updated 2026-06-16) adds that the 30-character CTA length is advice, not a hard limit. |
| `FlowReply` | not in the spec — webhook shape | Flows guide *Flows Webhooks*: `interactive.nfm_reply.{name, body, response_json}`, with `response_json` a JSON **document inside a string** and the flow token inside that document. |

## Deliberate divergences

* **Response envelopes are unwrapped permissively.** Where the spec and the
  reference disagree (profile, phone number, subscribed apps), the parser accepts
  all documented variants instead of picking one and failing on the other.
* **Unknown enum values are carried, not rejected.** `QualityRating::Unknown`,
  `MessagingLimitTier::Unknown` and the string-typed `vertical`, `name_status`
  and `account_mode` keep whatever Meta sends, so a new value cannot break a
  deployment.
* **No field-level webhook subscription.** The WABA node has no
  `subscribed_fields`; that parameter belongs to the app-level
  `/{app_id}/subscriptions` endpoint, which is out of scope (README).
* **Flow identifiers are bounded by this crate, not by Meta.** Neither
  `flow_token` (1024 bytes) nor a screen id (64 characters, `[A-Za-z0-9_-]`) has
  a documented maximum; the caps keep a request from carrying an unbounded
  identifier. Whether a well-formed screen exists in the flow is Meta's answer,
  not ours.
* **`nfm_reply` is decoded once.** `response_json` arrives stringified;
  `FlowReply` parses it and keeps the wire object in `FlowReply::raw`, so a
  malformed document degrades to that string instead of failing the webhook —
  and a parsed reply re-serialises to exactly what Meta sent.

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
* Phone-number registration / deregistration: `https://developers.facebook.com/docs/whatsapp/cloud-api/reference` (the `/{PHONE_NUMBER_ID}/register` and `/{PHONE_NUMBER_ID}/deregister` edges)
* Two-step verification PIN: `https://developers.facebook.com/docs/whatsapp/cloud-api/reference/two-step-verification`
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
| `register`, `register_with_backup` | `POST /{phone-number-id}/register` | `/{Version}/{Phone-Number-ID}/register` |
| `deregister` | `DELETE /{phone-number-id}/deregister` | `/{Version}/{Phone-Number-ID}/deregister` |
| `set_two_step_verification_pin` | `POST /{phone-number-id}` (body `{"pin":"…"}`) | `/{Version}/{Phone-Number-ID}` |
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
| `SuccessOutcome` | register / deregister / two-step responses (`success`) | `success` is documented as a boolean; missing, `1`/`0` and `"true"`/`"1"` also decode — see *unverified*, README. Unknown fields land in `extra`. |
| `VerificationPin` | two-step verification reference | exactly 6 ASCII digits, checked before allocation. `Debug`/`Display` are fixed redacted text; no getter, no `Default`, equality is constant time. |
| `RegistrationBackup` | register edge (`backup{password,data}`) | on-premises migration blob; the OpenAPI description does not pin the shape down — see *unverified*, README. |
| `RegisterRequest` / `TwoStepVerificationRequest` | register edge, two-step reference | register sends `messaging_product` + `pin` (+ optional `backup`); two-step sends `pin` alone. |
| `WebhookSubscriptions` / `SubscribedApp` | `SubscribedAppsResponse`, `SubscribedApp` | Each entry may wrap the app in `whatsapp_business_api_data` (spec example) — both parse. |
| `SubscriptionConfig` | `SubscriptionRequest` | `override_callback_uri`, `verify_token`. |
| `BusinessUseCaseUsage` / `UseCaseUsage` | header `x-business-use-case-usage` | Header value is a JSON object keyed by business id, each value an array of `{type, call_count, total_cputime, total_time, estimated_time_to_regain_access}`. |
| `SingleProductMessage` | `Interactive` with `type: product`, `ActionObject` variant `{catalog_id, product_retailer_id}` | `HeaderObject` says a header "cannot be set for 'product' type", so the struct has none. `BodyObject` is "optional for 'product' type"; footer optional. Body 1024, footer 60. |
| `MultiProductMessage` / `ProductSection` / `ProductItem` | `Interactive` with `type: product_list`, `ActionObject` variant `{catalog_id, sections}`, `SectionObject.product_items` | Header "required for 'product_list'", body "required for other interactive message types" — both are constructor arguments. `sections` 1–10, `product_items` 1–30 per section, section `title` max 24 and required once there is more than one section, header/footer 60, body 1024. Meta's reference caps the whole message at 30 products, so that cap applies on top of the per-section one. |

## Deliberate divergences

* **Response envelopes are unwrapped permissively.** Where the spec and the
  reference disagree (profile, phone number, subscribed apps), the parser accepts
  all documented variants instead of picking one and failing on the other.
* **Unknown enum values are carried, not rejected.** `QualityRating::Unknown`,
  `MessagingLimitTier::Unknown` and the string-typed `vertical`, `name_status`
  and `account_mode` keep whatever Meta sends, so a new value cannot break a
  deployment.
* **Two-step verification is set, never disabled.** `POST /{PHONE_NUMBER_ID}` with a `pin` is the only write the reference offers; there is no endpoint to turn two-step verification off, so this crate exposes `set_two_step_verification_pin` (rotation) and no `disable`.
* **The two-step body carries `pin` alone.** Unlike `register`, the reference does not send `messaging_product` there, so neither do we.
* **`deregister` is treated as non-idempotent even though it is a `DELETE`.** `Repeatability::of_method` calls `DELETE` idempotent, which is right for reads like `delete_media` but wrong here: a repeated deregistration is a state change the caller has to see, so `deregister` overrides the verb rule and a 5xx surfaces as `Error::Ambiguous` instead of being re-sent.
* **No field-level webhook subscription.** The WABA node has no
  `subscribed_fields`; that parameter belongs to the app-level
  `/{app_id}/subscriptions` endpoint, which is out of scope (README).
* **Required catalog fields live in the constructor.** For `product_list` the
  spec makes `header` and `body` mandatory and for `product` it forbids the
  header; `MultiProductMessage::new` therefore takes header and body, and
  `SingleProductMessage` has no header field. A shape the wire rejects cannot be
  built, so no runtime check has to remember it.
* **Two documented caps the spec omits are enforced.** The `SectionObject` caps
  `product_items` at 30 *per section*; Meta's reference caps a multi-product
  message at 30 products overall, and requires a `title` as soon as there is more
  than one section. Both are checked, because Meta rejects the payload either
  way and a client-side error names the field.
* **Catalog and retailer ids are not length-capped.** Neither source gives a
  maximum, so only emptiness and control characters are refused.

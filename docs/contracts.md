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
| `create_template` | `POST /{phone-number-id}/message_templates` | `/{Version}/{WABA-ID}/message_templates` (POST) |
| `update_template` | `POST /{phone-number-id}/message_templates` | same (`Edit template`) |
| `delete_template` | `DELETE /{phone-number-id}/message_templates?name=…` | same (`Delete template by name`) |
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
| `CatalogMessage` / `ProductMessage` / `ProductListMessage` | `InteractiveObject`, `ActionObject` | `catalog_message` uses `action.name = catalog_message` and `thumbnail_product_retailer_id`; `product` and `product_list` use `catalog_id` plus `product_retailer_id` or `sections`. Product lists require a text header. Limits: 10 sections, 30 products total, 24-character section titles, 60-character header/footer, 1024-byte body. |
| `TemplateDefinition` / `TemplateUpdate` | `Create template` and `Edit template` request bodies | `name`, `language` (string, not the send-side object), `category`, `components[]` with `HEADER`/`BODY`/`FOOTER`/`BUTTONS`. Header formats TEXT, IMAGE, VIDEO, DOCUMENT, LOCATION; button types QUICK_REPLY, URL, COPY_CODE, PHONE_NUMBER, OTP. Limits: name 512 bytes, language 32 characters, header/footer 60 characters, body 1024 bytes, button label 25 characters, URL 2000 characters, copy-code example 15 characters, 10 buttons (max 2 URL, max 1 copy-code), 20 examples per component, 4 components, code expiration 1–90 minutes. Authentication templates use `add_security_recommendation` instead of body text and `code_expiration_minutes` in the footer. |
| `TemplateAck` / `TemplateDeleteAck` | create, edit and delete responses | Create answers `{id, status, category}`, edit and delete `{ "success": true }`. Every field is optional and unknown members are kept. |
| `OrderDetailsMessage` / `OrderStatusMessage` | `InteractiveObject`, `ActionObject` examples | The existing order-details contract uses `order_details` with `action.name = review_order`, `reference_id`, and an order of `catalog_id` plus `product_items`. Status updates use `order_status` with the same action name and `order.status`. Limits: 30 items, 35-character reference id, quantity >= 1, finite non-negative prices, three-letter uppercase currency. |
| `Order` / `OrderItem` | `OrderMessage.order` | Inbound `type: "order"` items; snake_case and camelCase aliases both parse, as do numeric strings for `quantity` and `item_price`. Unknown members stay in `extra`. |
| `AddressMessage` / `AddressReply` | not present in the v23 OpenAPI file | Shape comes from the Cloud API address-message guide: `interactive.type = address_message`, `action.name = address_message`, parameters `country`, `values[]` and `saved_addresses[]`. Unknown field types and members are carried. |
| `FlowReply` / `FlowResponse` | not present in the v23 OpenAPI file | Shape comes from the flows-webhooks reference, `https://developers.facebook.com/docs/whatsapp/flows/reference/flowswebhooks/`: `interactive.type = "nfm_reply"`, and `interactive.nfm_reply` carries exactly three documented strings — `name`, `body`, `response_json`. The flow's answers, `flow_token` included, are *inside* `response_json` as a JSON document encoded in a string; there is no top-level token field. Decoding is fallible (`Error::Payload`) and the raw string is kept on the struct. |

## Deliberate divergences

* **Template management is phone-number scoped, and the edit is a POST.** The
  v23 OpenAPI file documents `POST`/`DELETE` on `/{WABA-ID}/message_templates`;
  the client addresses `/{phone_number_id}/message_templates` (the node it is
  bound to) with the same request bodies, as issue #10 asks for. The edit uses
  Meta's `Edit template` operation — a `POST` with `name` and `language`
  selecting the template. Issue #10 named a `PUT`; no `PUT` exists in the v23
  file or in the template-management guide, so the documented verb is used. A
  `PUT` on that edge would be rejected by the API.

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
* **Commerce order variants.** The v23 OpenAPI example also contains a newer
  `review_and_pay` payment flow. This crate implements the established
  `review_order` order-details/status contract; the payment-specific shape is
  not represented by `OrderDetailsMessage`.

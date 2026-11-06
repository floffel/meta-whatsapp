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
* Send / read a message: `https://developers.facebook.com/docs/whatsapp/cloud-api/reference/messages`
* Block users: `https://developers.facebook.com/docs/business-messaging/whatsapp/block-users`
* Phone-number registration / deregistration: `https://developers.facebook.com/docs/whatsapp/cloud-api/reference` (the `/{PHONE_NUMBER_ID}/register` and `/{PHONE_NUMBER_ID}/deregister` edges)
* Two-step verification PIN: `https://developers.facebook.com/docs/whatsapp/cloud-api/reference/two-step-verification`
* Quality rating background: `https://www.facebook.com/business/help/896873687365001`
* Order messages (inbound `type = order`):
  `https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/reference/messages/order/`
  — the `order` object (`catalog_id`, optional `text`, `product_items[]` with
  `product_retailer_id`, `quantity`, `item_price`, `currency`) and one example
  payload. The page types `item_price` as *Integer* while its own example sends
  `7.99`, so the crate reads a finite, non-negative number and refuses the rest.
* Address messages (India only, `interactive.type = address_message`):
  `https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/address-messages/`
  — field table, the four `action.parameters` members, and both the request and
  the `nfm_reply` webhook examples.
* Flows API: `https://developers.facebook.com/docs/whatsapp/flows/reference/flowsapi/`
  — the web-preview request (`GET /{FLOW-ID}?fields=preview.invalidate(…)`),
  its response, the 30-day expiry, and invalidation behavior. The `getFlow`
  operation and full response schema are in the v23 OpenAPI source above.

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
| `register`, `register_with_backup` | `POST /{phone-number-id}/register` | `/{Version}/{Phone-Number-ID}/register` |
| `deregister` | `DELETE /{phone-number-id}/deregister` | `/{Version}/{Phone-Number-ID}/deregister` |
| `set_two_step_verification_pin` | `POST /{phone-number-id}` (body `{"pin":"…"}`) | `/{Version}/{Phone-Number-ID}` |
| `message_status` | `GET /{phone-number-id}?id={wamid}` | `/{Version}/{Phone-Number-ID}` |
| `waba_phone_numbers`, `all_waba_phone_numbers` | `GET /{waba-id}/phone_numbers` with optional `fields`, `filtering`, `sort`, `limit`, `after`, `before` | `/{Version}/{WABA-ID}/phone_numbers` |
| `create_waba_phone_number` | `POST /{waba-id}/phone_numbers` | same |
| `request_phone_number_code` | `POST /{phone-number-id}/request_code` | `/{Version}/{Phone-Number-ID}/request_code` |
| `verify_phone_number_code` | `POST /{phone-number-id}/verify_code` | `/{Version}/{Phone-Number-ID}/verify_code` |
| `blocked_users` | `GET /{phone-number-id}/block_users` | `/{Version}/{Phone-Number-ID}/block_users` |
| `blocked_users_after`, `blocked_users_with`, `all_blocked_users`, `all_blocked_users_with` | same, plus `?limit=…&after=…` (both optional) | same |
| `block_users` | `POST /{phone-number-id}/block_users` | same |
| `unblock_users` | `DELETE /{phone-number-id}/block_users` | same |
| `webhook_subscriptions` | `GET /{waba-id}/subscribed_apps` | `/{Version}/{WABA-ID}/subscribed_apps` |
| `subscribe_webhooks` | `POST /{waba-id}/subscribed_apps` | same |
| `unsubscribe_webhooks` | `DELETE /{waba-id}/subscribed_apps` | same |
| `app_webhook_subscriptions` | `GET /{app-id}/subscriptions` | `/{Version}/{App-ID}/subscriptions` |
| `subscribe_app_webhooks` | `POST /{app-id}/subscriptions` | same; body has `object`, `callback_url`, `verify_token`, comma-separated `fields` |
| `flow` | `GET /{flow-id}?fields=…&date_format=…` | `/{Version}/{Flow-ID}` (`getFlow`) |
| `flow_preview` | `GET /{flow-id}?fields=preview.invalidate(false\|true)` | same (`getFlow`); preview also from the Flows API guide |

## Shapes and their sources

| Type | Spec schema | Notes |
| --- | --- | --- |
| `BusinessProfile` | `WhatsAppBusinessProfile`, `WhatsAppBusinessProfileResponse` | Limits: `about` 1–139, `address`/`description` 256, `email` 128, `websites` max 2 × 256. The response envelope differs between the reference and the spec example — see *unverified*, README. |
| `BusinessProfileUpdate` | `WhatsAppBusinessProfileUpdateRequest` | `messaging_product` is required and always sent; `profile_picture_handle` comes from the Resumable Upload API (not implemented here). |
| `PhoneNumberInfo` | `PhoneNumberInfo` | `quality_rating` ∈ {GREEN, YELLOW, RED, NA}; `name_status` ∈ {APPROVED, AVAILABLE_WITHOUT_REVIEW, DECLINED, EXPIRED, PENDING_REVIEW, NONE}; `code_verification_status` ∈ {VERIFIED, UNVERIFIED}. |
| `MessagingLimitTier` | `WhatsAppAccountNumber.messaging_limit_tier` | TIER_50, TIER_250, TIER_1K, TIER_10K, TIER_100K, TIER_UNLIMITED. |
| `BlockedUser` / `BlockedUsersPage` | `GetBlockedUsersData`, `BlockedUser` | `data[]` plus `paging`. |
| `BlockUsersQuery` | paging is described in the block-users guide, not typed in the OpenAPI file | `limit` (≥ 1; no documented maximum) plus the opaque `after` cursor from `paging.cursors.after`, rendered as `limit=…&after=…`. |
| `SubscriptionField` / `AppWebhookSubscriptionRequest` | app-level subscriptions guide | `SubscriptionField` covers `messages`, `message_template_status_update`, `phone_number_quality_update`, plus unknown names. Callback URL ≤ 1024 characters, verify token ≤ 256, and at least one field. |
| `AppWebhookSubscriptions` / `AppWebhookSubscription` | app-level subscriptions guide | `data[]` entries identify an `object`; WhatsApp entries carry the callback, active state and field list. Fields parse from either bare strings or `{name, version}` objects. Serde errors are retained as `Error::Json { source }`. |
| `BlockOutcome` | `BlockUsersData` (`added_users`), `UnblockUsersData` (`removed_users`), `BlockedUserOperation` | Same payload shape for POST and DELETE; ids travel in the body. |
| `SuccessOutcome` | register / deregister / two-step responses (`success`) | `success` is documented as a boolean; missing, `1`/`0` and `"true"`/`"1"` also decode — see *unverified*, README. Unknown fields land in `extra`. |
| `VerificationPin` | two-step verification reference | exactly 6 ASCII digits, checked before allocation. `Debug`/`Display` are fixed redacted text; no getter, no `Default`, equality is constant time. |
| `RegistrationBackup` | register edge (`backup{password,data}`) | on-premises migration blob; the OpenAPI description does not pin the shape down — see *unverified*, README. |
| `RegisterRequest` / `TwoStepVerificationRequest` | register edge, two-step reference | register sends `messaging_product` + `pin` (+ optional `backup`); two-step sends `pin` alone. |
| `PhoneNumberCreateRequest` / `PhoneNumberCreateOutcome` | `PhoneNumberCreateRequest`, `PhoneNumberCreateResponse` | `phone_number` is 7–15 E.164 digits starting with 1–9 and no `+`; `verified_name` is 2–75 characters (with control characters refused locally); optional `cc` is 1–3 digits; `migrate_phone_number` and `preverified_id` are opt-in. The response requires a non-empty `id`. |
| `WabaPhoneNumber` / `WabaPhoneNumbersPage` | `WhatsAppBusinessAccountPhoneNumber`, `WhatsAppBusinessAccountPhoneNumbersConnection` | The documented response fields are typed; status, certification and platform strings remain strings for forward compatibility. Unknown fields stay in `extra`; `data[]` plus `paging.cursors.after` drives the list walk. |
| `PhoneNumberQuery` / `PhoneNumberFilter` / `PhoneNumberSort` | parameters of `getWhatsAppBusinessAccountPhoneNumbers` | `limit` is 1–100; `after` and `before` are mutually exclusive; fields use the existing safe field-list grammar; the only documented operator is `EQUAL`; filterable fields are `account_mode`, `messaging_limit_tier` and `is_official_business_account`. Filters are a JSON array encoded as the `filtering` query value. |
| `RequestCodeRequest` / `RequestCodeOutcome` | `RequestCodeRequest`, `RequestCodeResponse` | Exactly `{code_method: SMS|VOICE, language: ll_CC}`; `success` is required in the response schema and is a boolean here. |
| `VerificationCode` / `VerifyCodeRequest` / `VerifyCodeOutcome` | `VerifyCodeRequest`, `VerifyCodeResponse` | Exactly six ASCII digits, no getter/default, constant-time equality and redacted `Debug`/`Display`; request body is `{code}`. Response has `success` and optional verified `id`. The code is not the registration PIN. |
| `WebhookSubscriptions` / `SubscribedApp` | `SubscribedAppsResponse`, `SubscribedApp` | Each entry may wrap the app in `whatsapp_business_api_data` (spec example) — both parse. |
| `SubscriptionConfig` | `SubscriptionRequest` | `override_callback_uri`, `verify_token`. |
| `Order` / `OrderItem` | not in the v23 OpenAPI file — parse-side only, from the *Order messages* webhook reference | Inbound `type: "order"` message. `catalog_id` string, optional `text`, `product_items[]` with `product_retailer_id`, `quantity`, `item_price`, `currency`. The reference publishes no maximum for the array, the quantity, the text or the price, so none is capped; `quantity` is a `u32` (number or decimal string) and `item_price` a finite, non-negative number, and a member that is neither stays verbatim in `extra` instead of becoming a zero. `Order::new` / `OrderItem::new` plus `validate` are the mirror image for an application that has to hand a parsed order on: at least one line, `quantity >= 1`, a finite non-negative price, a three-letter ISO-4217 currency — and no ceiling on the item count or on `text`, because the reference publishes none and its own example sends `"text": ""`. Outbound order messages (`order_details`, `order_status`) are interactive and not implemented — issue #20. |
| `AddressMessage` / `AddressValues` / `SavedAddress` / `AddressValidationError` | `interactive` with `type` and `action.name` = `address_message` | Values in the guide's field table; `in_pin_code` max 6 characters is the only per-field limit it publishes. `country` is required (ISO 3166-1 alpha-2) and is an action parameter, not a value. `validation_errors` is a field → message object. The number of `saved_addresses` is not bounded — see *unverified*, README. |
| `AddressReply` | `interactive.nfm_reply.response_json` (a JSON string) | `{"saved_address_id": …, "values": {…}}` from the guide's submission example; unknown keys stay in `extra`. |
| `BusinessUseCaseUsage` / `UseCaseUsage` | header `x-business-use-case-usage` | Header value is a JSON object keyed by business id, each value an array of `{type, call_count, total_cputime, total_time, estimated_time_to_regain_access}`. |
| `SingleProductMessage` | `Interactive` with `type: product`, `ActionObject` variant `{catalog_id, product_retailer_id}` | `HeaderObject` says a header "cannot be set for 'product' type", so the struct has none. `BodyObject` is "optional for 'product' type"; footer optional. Body 1024, footer 60. |
| `MultiProductMessage` / `ProductSection` / `ProductItem` | `Interactive` with `type: product_list`, `ActionObject` variant `{catalog_id, sections}`, `SectionObject.product_items` | Header "required for 'product_list'", body "required for other interactive message types" — both are constructor arguments. `sections` 1–10, `product_items` 1–30 per section, section `title` max 24 and required once there is more than one section, header/footer 60, body 1024. Meta's reference caps the whole message at 30 products, so that cap applies on top of the per-section one. |
| `CatalogMessage` | `Interactive` with `type: catalog_message`, `ActionObject` variant `{name: catalog_message, parameters.thumbnail_product_retailer_id}` (the spec's "Send Catalog Message" example) | The action carries exactly one parameter, the thumbnail product. `BodyObject` — "optional for 'product' type, required for other interactive message types" — is a constructor argument and 1024 bytes; `footer` is optional and 60 characters. No header: the example sends none and the type exists to open the catalog, not to carry media. |
| `FlowReply` / `FlowResponse` | not present in the v23 OpenAPI file | Shape comes from the flows-webhooks reference, `https://developers.facebook.com/docs/whatsapp/flows/reference/flowswebhooks/`: `interactive.type = "nfm_reply"`, and `interactive.nfm_reply` carries exactly three documented strings — `name`, `body`, `response_json`. The flow's answers, `flow_token` included, are *inside* `response_json` as a JSON document encoded in a string; there is no top-level token field. Decoding is fallible (`Error::Payload`) and the raw string is kept on the struct. |
| `FlowGetRequest` / `FlowPreviewRequest` | `getFlow` query parameters in the v23 OpenAPI file plus the Flows API preview guide | `fields` and `date_format` are optional. The preview guide sets `fields=preview.invalidate(false)` and documents `true` as invalidation. Flow ids and field expressions are validated before URL construction; the 256-byte id cap is crate-side because the spec gives no maximum. |
| `Flow` / `FlowPreviewResponse` | `getFlow` response and the guide's *Get Preview URL* example | `Flow` models `id`, `name`, `status`, `categories`, `json_version`, `data_api_version`, `data_channel_uri`, `endpoint_uri`, `preview`, `validation_errors`, `health_status`, `whatsapp_business_account` and `application`; metrics and future fields stay in `extra`. `FlowPreviewResponse` is `{id, preview}`. Every member defaults because field selection makes the response sparse. The guide's `DRAFT`, `PUBLISHED`, `DEPRECATED`, `BLOCKED` and `THROTTLED` statuses are typed; unknown statuses are retained. `preview.expires_at` accepts the documented date string and the `date_format=U` number. |
| `FlowValidationError` / `FlowValidationPointer` | validate/update Flow JSON examples in the Flows API guide and `getFlow` schema | The schema leaves `validation_errors[]` untyped, while its examples are objects with `error`, `error_type`, `message`, line/column ranges and pointers. Objects are typed; scalar members are retained under `extra.value` so one legacy entry cannot make the Flow unreadable. |
| `FlowHealthStatus` / `FlowHealthEntity` / `FlowHealthError` | `getFlow` response schema | `can_send_message` is kept as a string because the schema gives examples (`AVAILABLE`, `LIMITED`, `BLOCKED`) but no enum. Entities and their errors are typed; unknown members remain in `extra`. |
| `TemplateDefinition` / `TemplateUpdate` | `Create template` and `Edit template` request bodies | `name`, `language` (string, not the send-side object), `category`, `components[]` with `HEADER`/`BODY`/`FOOTER`/`BUTTONS`. Header formats TEXT, IMAGE, VIDEO, DOCUMENT, LOCATION; button types QUICK_REPLY, URL, COPY_CODE, PHONE_NUMBER, OTP. Limits: name 512 bytes, language 32 characters, header/footer 60 characters, body 1024 bytes, button label 25 characters, URL 2000 characters, copy-code example 15 characters, 10 buttons (max 2 URL, max 1 copy-code), 20 examples per component, 4 components, code expiration 1-90 minutes. Authentication templates use `add_security_recommendation` instead of body text and `code_expiration_minutes` in the footer. |
| `TemplateAck` / `TemplateDeleteAck` | create, edit and delete responses | Create answers `{id, status, category}`, edit and delete `{ "success": true }`. Every field is optional and unknown members are kept. |
| `MessageStatus` / `DeliveryStatus` | *Read a Message* response, `messages[]` | `id`, `status` ∈ {sent, delivered, read, failed}, `timestamp`, `recipient_id`, `errors[]` — the same field set the `statuses` webhook pushes as `Status`, read on demand instead. An unlisted status string is carried as `DeliveryStatus::Unknown`. The reference gives no format or length for a wamid, so `MAX_WAMID_LEN` (256 bytes) is this crate's own bound on what may enter the query string. |

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
* **Two-step verification is set, never disabled.** `POST /{PHONE_NUMBER_ID}` with a `pin` is the only write the reference offers; there is no endpoint to turn two-step verification off, so this crate exposes `set_two_step_verification_pin` (rotation) and no `disable`.
* **The two-step body carries `pin` alone.** Unlike `register`, the reference does not send `messaging_product` there, so neither do we.
* **`deregister` is treated as non-idempotent even though it is a `DELETE`.** `Repeatability::of_method` calls `DELETE` idempotent, which is right for reads like `delete_media` but wrong here: a repeated deregistration is a state change the caller has to see, so `deregister` overrides the verb rule and a 5xx surfaces as `Error::Ambiguous` instead of being re-sent.
* **A non-string `status` is carried too.** `MessageStatus` decodes any JSON
  shape for `status`: a string is parsed, `null` becomes the default unknown,
  anything else is kept as its raw JSON text. Failing the whole response on one
  unexpected field value would hide the delivery state the caller asked for.
* **Transport deadlines are part of the delivery contract.** The bundled
  `ReqwestTransport` sets a 60-second total request deadline
  (`DEFAULT_REQUEST_TIMEOUT`) and a 10-second connection deadline
  (`DEFAULT_CONNECT_TIMEOUT`). A `Request::timeout` overrides the total deadline
  for that one call. A total timeout after the write is
  `TransportPhase::AfterWrite`: the request may have taken effect, so a POST is
  surfaced as ambiguous and never retried automatically. A connection timeout
  is `BeforeWrite` and safe to retry.
* **Transport redirects are disabled.** `ReqwestTransport` uses
  `reqwest::redirect::Policy::none()`. The documented Graph endpoints are
  absolute URLs on one host and do not redirect; following a `3xx` could replay
  a mutating POST (`307`/`308`) or turn it into a GET (`301`/`302`). The original
  `3xx` is surfaced as `Error::Api`, so the retry layer sees one request and one
  terminal response.
* **Commerce numbers are read permissively, never guessed.** An inbound order
  `quantity` or `item_price` is accepted as a JSON number or as a plain decimal
  string; a member that is neither (a `NaN`, a negative price, an object) stays
  verbatim in the item's `extra` and the field stays empty, so an unreadable
  price never turns into a zero. Unknown keys survive in the matching `extra`
  map, including through a render/parse round trip.
* **App-level field selection is separate from WABA attachment.** The WABA node
  has no `subscribed_fields`; that parameter belongs to the app-level
  `/{app_id}/subscriptions` endpoint implemented by `app_webhook_subscriptions`
  and `subscribe_app_webhooks`. The WABA-level calls still attach or detach the
  app itself.
* **Phone-number onboarding keeps one registration path.** The WABA create edge
  provisions and returns the number id; `request_code` and `verify_code` prove
  ownership; the existing `POST /{phone-number-id}/register` remains the final
  step. No second WABA-level registration wrapper is added. Both verification
  codes and PINs are secrets but distinct types, and verification happens before
  registration as Meta documents.
* **The address family is reference-only.** `address_message` does not appear in
  the v23.0 OpenAPI description at all — it is modelled from the reference
  page's field table and examples. That page scopes the feature to India and
  lists only the Indian fields; the Singapore-only names (`sg_post_code`,
  `unit_number`) survive only in the On-Premises guide, which Meta sunset on
  2025-10-23, so they are not modelled. A `country` is checked for shape, not
  against that list: an unsupported market is Meta's decision to reject, and
  guessing the list would turn a future market into a local failure.
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
* **`flow_message_version` is fixed at the documented `"3"`.** The issue that
  requested `FlowMessage` listed the version as a settable field; it is instead
  emitted as the value Meta's own examples use, because a caller-chosen version
  can only be wrong on the wire. Nothing in the docs shows another supported
  value, so making it an option would invent a knob.

//! Address messages — the `interactive` payload that asks a customer for a
//! delivery address, and the typed form of the reply.
//!
//! Meta documents this family for businesses in India only. It is an
//! *interactive* message whose `type` and `action.name` are both
//! `address_message`; the address itself is described by the action parameters
//! `country` (required, ISO 3166-1 alpha-2), `values` (pre-filled fields),
//! `saved_addresses` (addresses the customer may pick instead of typing) and
//! `validation_errors` (field → message, so the form can be sent back with the
//! problem marked).
//!
//! The customer's answer arrives as an `nfm_reply` webhook whose
//! `response_json` string carries `saved_address_id` and `values`;
//! [`AddressReply`] parses it into [`AddressValues`].
//!
//! ```
//! use meta_whatsapp::{
//!     AddressMessage, AddressReply, AddressValues, MessageRequest, SavedAddress,
//! };
//!
//! let request = AddressMessage::new("IN", "Where should the order go?")
//!     .values(AddressValues::new().name("Ada Lovelace").city("Mumbai"))
//!     .saved_address(SavedAddress::new(
//!         "address1",
//!         AddressValues::new().city("Pune").in_pin_code("411001"),
//!     ));
//! let json = MessageRequest::new("919876543210", request).to_json()?;
//! assert_eq!(json["interactive"]["type"], "address_message");
//! assert_eq!(json["interactive"]["action"]["name"], "address_message");
//! assert_eq!(json["interactive"]["action"]["parameters"]["country"], "IN");
//!
//! let reply = AddressReply::parse(
//!     r#"{"saved_address_id":"address1","values":{"city":"Pune","in_pin_code":"411001"}}"#,
//! )?;
//! assert_eq!(reply.saved_address_id.as_deref(), Some("address1"));
//! assert_eq!(reply.values.city.as_deref(), Some("Pune"));
//! # Ok::<(), meta_whatsapp::Error>(())
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::message::{
    require_max_bytes, require_max_chars, require_no_controls, require_non_empty, HeaderText,
    CAPTION_MAX_BYTES, HEADER_MAX_CHARS,
};
use crate::types::{FlowReply, Message};

/// Length of a country code: ISO 3166-1 alpha-2, the format Meta requires.
///
/// ```
/// use meta_whatsapp::ADDRESS_COUNTRY_LEN;
/// assert_eq!(ADDRESS_COUNTRY_LEN, 2);
/// ```
pub const ADDRESS_COUNTRY_LEN: usize = 2;

/// Maximum length of `in_pin_code`, in characters — the only per-field limit
/// Meta publishes for the address form.
///
/// ```
/// use meta_whatsapp::ADDRESS_PIN_CODE_MAX_CHARS;
/// assert_eq!(ADDRESS_PIN_CODE_MAX_CHARS, 6);
/// ```
pub const ADDRESS_PIN_CODE_MAX_CHARS: usize = 6;

/// Ceiling for one free-form address field, in UTF-8 bytes.
///
/// Meta publishes no limit for the other fields. The crate caps them so a typo
/// or a copy-paste accident cannot put a megabyte-long string on the wire; the
/// value matches [`crate::message::FREEFORM_MAX_BYTES`], which already bounds
/// the other free-form message fields.
///
/// ```
/// use meta_whatsapp::ADDRESS_FIELD_MAX_BYTES;
/// assert_eq!(ADDRESS_FIELD_MAX_BYTES, 1024);
/// ```
pub const ADDRESS_FIELD_MAX_BYTES: usize = 1024;

/// The shortest phone number the crate accepts, in digits. E.164 allows at most
/// 15 and publishes no minimum; seven is the shortest length any national
/// numbering plan uses for a subscriber number.
const PHONE_MIN_DIGITS: usize = 7;

/// The longest phone number E.164 allows, in digits.
const PHONE_MAX_DIGITS: usize = 15;

/// One address form: the documented fields, each optional, plus anything the
/// API adds later.
///
/// Meta's field names are not the ones the feature is usually described with:
/// `zip` is `in_pin_code`, `street` is `address`, and `country` is not a value
/// field at all — it is required next to this object, see
/// [`AddressMessage::new`]. Empty strings are legal (Meta's own example sends
/// `"building_name": ""`), so only the length is checked.
///
/// ```
/// use meta_whatsapp::AddressValues;
///
/// let values = AddressValues::new().city("Mumbai").in_pin_code("400063");
/// assert_eq!(values.city.as_deref(), Some("Mumbai"));
/// assert!(!values.is_empty());
/// assert!(AddressValues::new().is_empty());
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AddressValues {
    /// Recipient name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Phone number, optional `+` and 7–15 digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
    /// Postal code (the field usually called `zip`), at most
    /// [`ADDRESS_PIN_CODE_MAX_CHARS`] characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_pin_code: Option<String>,
    /// Flat or house number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub house_number: Option<String>,
    /// Floor number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_number: Option<String>,
    /// Tower number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tower_number: Option<String>,
    /// Building or apartment name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub building_name: Option<String>,
    /// Street line (the field usually called `street`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Landmark or area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landmark_area: Option<String>,
    /// City.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    /// State.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl AddressValues {
    /// An empty form.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert!(AddressValues::new().name.is_none());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the recipient name.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert_eq!(AddressValues::new().name("Ada").name.as_deref(), Some("Ada"));
    /// ```
    #[must_use]
    pub fn name(mut self, value: impl Into<String>) -> Self {
        self.name = Some(value.into());
        self
    }

    /// Sets the phone number.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    ///
    /// assert!(AddressValues::new().phone_number("+911234567890").validate().is_ok());
    /// assert!(AddressValues::new().phone_number("+91 1234").validate().is_err());
    /// ```
    #[must_use]
    pub fn phone_number(mut self, value: impl Into<String>) -> Self {
        self.phone_number = Some(value.into());
        self
    }

    /// Sets the postal code.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert_eq!(
    ///     AddressValues::new().in_pin_code("400063").in_pin_code.as_deref(),
    ///     Some("400063")
    /// );
    /// ```
    #[must_use]
    pub fn in_pin_code(mut self, value: impl Into<String>) -> Self {
        self.in_pin_code = Some(value.into());
        self
    }

    /// Sets the house or flat number.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert_eq!(
    ///     AddressValues::new().house_number("12B").house_number.as_deref(),
    ///     Some("12B")
    /// );
    /// ```
    #[must_use]
    pub fn house_number(mut self, value: impl Into<String>) -> Self {
        self.house_number = Some(value.into());
        self
    }

    /// Sets the floor.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert_eq!(
    ///     AddressValues::new().floor_number("8").floor_number.as_deref(),
    ///     Some("8")
    /// );
    /// ```
    #[must_use]
    pub fn floor_number(mut self, value: impl Into<String>) -> Self {
        self.floor_number = Some(value.into());
        self
    }

    /// Sets the tower.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert_eq!(
    ///     AddressValues::new().tower_number("B").tower_number.as_deref(),
    ///     Some("B")
    /// );
    /// ```
    #[must_use]
    pub fn tower_number(mut self, value: impl Into<String>) -> Self {
        self.tower_number = Some(value.into());
        self
    }

    /// Sets the building or apartment name.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert_eq!(
    ///     AddressValues::new().building_name("Cello Triumph").building_name.as_deref(),
    ///     Some("Cello Triumph")
    /// );
    /// ```
    #[must_use]
    pub fn building_name(mut self, value: impl Into<String>) -> Self {
        self.building_name = Some(value.into());
        self
    }

    /// Sets the street line.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert_eq!(
    ///     AddressValues::new().address("Wing A, IB Patel Rd").address.as_deref(),
    ///     Some("Wing A, IB Patel Rd")
    /// );
    /// ```
    #[must_use]
    pub fn address(mut self, value: impl Into<String>) -> Self {
        self.address = Some(value.into());
        self
    }

    /// Sets the landmark or area.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert_eq!(
    ///     AddressValues::new().landmark_area("Goregaon").landmark_area.as_deref(),
    ///     Some("Goregaon")
    /// );
    /// ```
    #[must_use]
    pub fn landmark_area(mut self, value: impl Into<String>) -> Self {
        self.landmark_area = Some(value.into());
        self
    }

    /// Sets the city.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert_eq!(AddressValues::new().city("Delhi").city.as_deref(), Some("Delhi"));
    /// ```
    #[must_use]
    pub fn city(mut self, value: impl Into<String>) -> Self {
        self.city = Some(value.into());
        self
    }

    /// Sets the state.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    /// assert_eq!(
    ///     AddressValues::new().state("Maharashtra").state.as_deref(),
    ///     Some("Maharashtra")
    /// );
    /// ```
    #[must_use]
    pub fn state(mut self, value: impl Into<String>) -> Self {
        self.state = Some(value.into());
        self
    }

    /// Reads one documented field by its wire name (`city`, `in_pin_code`, …).
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    ///
    /// let values = AddressValues::new().city("Pune");
    /// assert_eq!(values.get("city"), Some("Pune"));
    /// assert_eq!(values.get("state"), None);
    /// assert_eq!(values.get("nickname"), None);
    /// ```
    #[must_use]
    pub fn get(&self, field: &str) -> Option<&str> {
        self.fields()
            .into_iter()
            .find(|(name, _, _)| *name == field)
            .and_then(|(_, _, value)| value)
            .map(String::as_str)
    }

    /// `true` when no documented field and no extra key is set.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    ///
    /// assert!(AddressValues::new().is_empty());
    /// assert!(!AddressValues::new().city("Pune").is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields().iter().all(|(_, _, value)| value.is_none()) && self.extra.is_empty()
    }

    /// Checks every documented limit before the value reaches the wire.
    ///
    /// ```
    /// use meta_whatsapp::AddressValues;
    ///
    /// assert!(AddressValues::new().city("Pune").validate().is_ok());
    /// assert!(AddressValues::new().in_pin_code("4000631").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the first offending field: an over-long
    /// value, an over-long `in_pin_code`, or a `phone_number` that is not an
    /// optional `+` followed by 7–15 digits.
    pub fn validate(&self) -> Result<()> {
        for (name, path, value) in self.fields() {
            let Some(value) = value else { continue };
            if name == "in_pin_code" {
                require_max_chars(path, value, ADDRESS_PIN_CODE_MAX_CHARS)?;
            }
            if name == "phone_number" {
                require_phone_number(path, value)?;
            }
            require_max_bytes(path, value, ADDRESS_FIELD_MAX_BYTES)?;
        }
        Ok(())
    }

    /// The documented fields as `(wire name, error path, value)`.
    fn fields(&self) -> [(&'static str, &'static str, Option<&String>); 11] {
        [
            (
                "name",
                "interactive.action.parameters.values.name",
                self.name.as_ref(),
            ),
            (
                "phone_number",
                "interactive.action.parameters.values.phone_number",
                self.phone_number.as_ref(),
            ),
            (
                "in_pin_code",
                "interactive.action.parameters.values.in_pin_code",
                self.in_pin_code.as_ref(),
            ),
            (
                "house_number",
                "interactive.action.parameters.values.house_number",
                self.house_number.as_ref(),
            ),
            (
                "floor_number",
                "interactive.action.parameters.values.floor_number",
                self.floor_number.as_ref(),
            ),
            (
                "tower_number",
                "interactive.action.parameters.values.tower_number",
                self.tower_number.as_ref(),
            ),
            (
                "building_name",
                "interactive.action.parameters.values.building_name",
                self.building_name.as_ref(),
            ),
            (
                "address",
                "interactive.action.parameters.values.address",
                self.address.as_ref(),
            ),
            (
                "landmark_area",
                "interactive.action.parameters.values.landmark_area",
                self.landmark_area.as_ref(),
            ),
            (
                "city",
                "interactive.action.parameters.values.city",
                self.city.as_ref(),
            ),
            (
                "state",
                "interactive.action.parameters.values.state",
                self.state.as_ref(),
            ),
        ]
    }

    /// Renders the set fields into the `values` object, keeping `extra` keys.
    fn to_json(&self) -> Value {
        let mut object = Map::new();
        for (name, _, value) in self.fields() {
            if let Some(value) = value {
                object.insert(name.to_string(), Value::String(value.clone()));
            }
        }
        for (key, value) in &self.extra {
            object.insert(key.clone(), value.clone());
        }
        Value::Object(object)
    }
}

/// One address the customer may pick instead of typing a new one.
///
/// ```
/// use meta_whatsapp::{AddressValues, SavedAddress};
///
/// let saved = SavedAddress::new("address1", AddressValues::new().city("Mumbai"));
/// assert_eq!(saved.id, "address1");
/// assert!(saved.validate().is_ok());
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SavedAddress {
    /// Id echoed back as `saved_address_id` when the customer picks this entry.
    #[serde(default)]
    pub id: String,
    /// The address it stands for; must prefill at least one field.
    #[serde(default)]
    pub value: AddressValues,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SavedAddress {
    /// Pairs an id with an address.
    ///
    /// ```
    /// use meta_whatsapp::{AddressValues, SavedAddress};
    ///
    /// let saved = SavedAddress::new("home", AddressValues::new().city("Pune"));
    /// assert_eq!(saved.value.city.as_deref(), Some("Pune"));
    /// ```
    pub fn new(id: impl Into<String>, value: AddressValues) -> Self {
        Self {
            id: id.into(),
            value,
            extra: Map::new(),
        }
    }

    /// Requires a usable id and a non-empty, valid address.
    ///
    /// ```
    /// use meta_whatsapp::{AddressValues, SavedAddress};
    ///
    /// assert!(SavedAddress::new("", AddressValues::new().city("Pune")).validate().is_err());
    /// assert!(SavedAddress::new("home", AddressValues::new()).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or oversized id, an id with control
    /// characters, an address with no field set, or a field limit from
    /// [`AddressValues::validate`].
    pub fn validate(&self) -> Result<()> {
        require_non_empty("interactive.action.parameters.saved_addresses.id", &self.id)?;
        require_no_controls("interactive.action.parameters.saved_addresses.id", &self.id)?;
        require_max_bytes(
            "interactive.action.parameters.saved_addresses.id",
            &self.id,
            ADDRESS_FIELD_MAX_BYTES,
        )?;
        if self.value.is_empty() {
            return Err(Error::validation(
                "interactive.action.parameters.saved_addresses.value",
                "must prefill at least one address field",
            ));
        }
        self.value.validate()
    }

    fn to_json(&self) -> Value {
        json!({ "id": self.id, "value": self.value.to_json() })
    }
}

/// One field-level error, sent back so the form can mark the problem.
///
/// ```
/// use meta_whatsapp::AddressValidationError;
///
/// let error = AddressValidationError::new("in_pin_code", "We could not locate this pin code.");
/// assert!(error.validate().is_ok());
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AddressValidationError {
    /// Address field the error belongs to (`in_pin_code`, `city`, …).
    #[serde(default)]
    pub field: String,
    /// Message shown next to that field.
    #[serde(default)]
    pub message: String,
}

impl AddressValidationError {
    /// Pairs a field with its message.
    ///
    /// ```
    /// use meta_whatsapp::AddressValidationError;
    ///
    /// let error = AddressValidationError::new("city", "Unknown city");
    /// assert_eq!(error.field, "city");
    /// ```
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }

    /// Requires a named field and a non-empty message.
    ///
    /// ```
    /// use meta_whatsapp::AddressValidationError;
    ///
    /// assert!(AddressValidationError::new("city", "").validate().is_err());
    /// assert!(AddressValidationError::new("", "Unknown").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or oversized field name, a field name
    /// with control characters, or an empty or oversized message.
    pub fn validate(&self) -> Result<()> {
        require_non_empty(
            "interactive.action.parameters.validation_errors.field",
            &self.field,
        )?;
        require_no_controls(
            "interactive.action.parameters.validation_errors.field",
            &self.field,
        )?;
        require_max_bytes(
            "interactive.action.parameters.validation_errors.field",
            &self.field,
            ADDRESS_FIELD_MAX_BYTES,
        )?;
        require_non_empty(
            "interactive.action.parameters.validation_errors.message",
            &self.message,
        )?;
        require_max_bytes(
            "interactive.action.parameters.validation_errors.message",
            &self.message,
            ADDRESS_FIELD_MAX_BYTES,
        )
    }
}

/// The `address_message` interactive message: a form that collects one
/// delivery address.
///
/// `country` is required by the API — a payload without it is rejected by Meta
/// with a validation error, so the crate refuses it first. Everything else is
/// optional: pre-filled `values`, a list of `saved_addresses` to pick from, and
/// `validation_errors` to resend the form with problems marked.
///
/// ```
/// use meta_whatsapp::{AddressMessage, AddressValidationError, AddressValues, SavedAddress};
///
/// let message = AddressMessage::new("IN", "Thanks for your order!")
///     .values(AddressValues::new().in_pin_code("666666"))
///     .saved_address(SavedAddress::new("address1", AddressValues::new().city("Mumbai")))
///     .validation_error(AddressValidationError::new(
///         "in_pin_code",
///         "We could not locate this pin code.",
///     ));
/// assert!(message.validate().is_ok());
/// assert_eq!(message.country, "IN");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressMessage {
    /// Optional text header.
    pub header: Option<HeaderText>,
    /// Body text, at most [`CAPTION_MAX_BYTES`] bytes.
    pub body: String,
    /// Optional footer, at most [`HEADER_MAX_CHARS`] characters.
    pub footer: Option<String>,
    /// ISO 3166-1 alpha-2 country code the form is for (Meta documents `IN`).
    pub country: String,
    /// Pre-filled fields.
    ///
    /// Boxed so that `MessageBody` stays small; read it as if it were a plain
    /// [`AddressValues`] (`message.values.city`).
    pub values: Box<AddressValues>,
    /// Addresses the customer may pick instead of typing.
    pub saved_addresses: Vec<SavedAddress>,
    /// Field errors, if this is a re-send after a failed validation.
    pub validation_errors: Vec<AddressValidationError>,
}

impl AddressMessage {
    /// Builds an address form for `country` with a body prompt.
    ///
    /// ```
    /// use meta_whatsapp::AddressMessage;
    ///
    /// let message = AddressMessage::new("IN", "Where should we deliver?");
    /// assert_eq!(message.body, "Where should we deliver?");
    /// assert!(message.values.is_empty());
    /// ```
    pub fn new(country: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            header: None,
            body: body.into(),
            footer: None,
            country: country.into(),
            values: Box::default(),
            saved_addresses: Vec::new(),
            validation_errors: Vec::new(),
        }
    }

    /// Adds a text header.
    ///
    /// ```
    /// use meta_whatsapp::{AddressMessage, HeaderText};
    /// assert!(AddressMessage::new("IN", "b").header(HeaderText::new("Order 42")).header.is_some());
    /// ```
    #[must_use]
    pub fn header(mut self, header: HeaderText) -> Self {
        self.header = Some(header);
        self
    }

    /// Adds a footer.
    ///
    /// ```
    /// use meta_whatsapp::AddressMessage;
    /// assert_eq!(AddressMessage::new("IN", "b").footer("Delivery only").footer.as_deref(), Some("Delivery only"));
    /// ```
    #[must_use]
    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Replaces the pre-filled fields.
    ///
    /// ```
    /// use meta_whatsapp::{AddressMessage, AddressValues};
    ///
    /// let message = AddressMessage::new("IN", "b").values(AddressValues::new().city("Delhi"));
    /// assert_eq!(message.values.city.as_deref(), Some("Delhi"));
    /// ```
    #[must_use]
    pub fn values(mut self, values: AddressValues) -> Self {
        self.values = Box::new(values);
        self
    }

    /// Appends one saved address.
    ///
    /// ```
    /// use meta_whatsapp::{AddressMessage, AddressValues, SavedAddress};
    ///
    /// let message = AddressMessage::new("IN", "b")
    ///     .saved_address(SavedAddress::new("home", AddressValues::new().city("Pune")));
    /// assert_eq!(message.saved_addresses.len(), 1);
    /// ```
    #[must_use]
    pub fn saved_address(mut self, saved: SavedAddress) -> Self {
        self.saved_addresses.push(saved);
        self
    }

    /// Appends one field error.
    ///
    /// ```
    /// use meta_whatsapp::{AddressMessage, AddressValidationError};
    ///
    /// let message = AddressMessage::new("IN", "b")
    ///     .validation_error(AddressValidationError::new("city", "Unknown city"));
    /// assert_eq!(message.validation_errors.len(), 1);
    /// ```
    #[must_use]
    pub fn validation_error(mut self, error: AddressValidationError) -> Self {
        self.validation_errors.push(error);
        self
    }

    /// Checks the country, the text parts and every parameter.
    ///
    /// ```
    /// use meta_whatsapp::AddressMessage;
    ///
    /// assert!(AddressMessage::new("IN", "Where?").validate().is_ok());
    /// assert!(AddressMessage::new("IND", "Where?").validate().is_err());
    /// assert!(AddressMessage::new("IN", "").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the first offending field: a country that is
    /// not two ASCII letters, an empty or oversized body, an oversized header or
    /// footer, a field limit from [`AddressValues::validate`], a bad saved
    /// address or validation error, or a duplicate saved-address id / duplicate
    /// `validation_errors` field.
    pub fn validate(&self) -> Result<()> {
        if self.country.len() != ADDRESS_COUNTRY_LEN
            || !self.country.bytes().all(|byte| byte.is_ascii_alphabetic())
        {
            return Err(Error::validation(
                "interactive.action.parameters.country",
                format!("must be a {ADDRESS_COUNTRY_LEN}-letter ISO 3166-1 alpha-2 country code"),
            ));
        }
        require_non_empty("interactive.body.text", &self.body)?;
        require_max_bytes("interactive.body.text", &self.body, CAPTION_MAX_BYTES)?;
        if let Some(header) = &self.header {
            header.validate()?;
        }
        if let Some(footer) = &self.footer {
            require_max_chars("interactive.footer.text", footer, HEADER_MAX_CHARS)?;
        }
        self.values.validate()?;

        let mut ids: Vec<&str> = Vec::with_capacity(self.saved_addresses.len());
        for saved in &self.saved_addresses {
            saved.validate()?;
            if ids.contains(&saved.id.as_str()) {
                return Err(Error::validation(
                    "interactive.action.parameters.saved_addresses.id",
                    "must be unique within one address message",
                ));
            }
            ids.push(&saved.id);
        }

        let mut fields: Vec<&str> = Vec::with_capacity(self.validation_errors.len());
        for error in &self.validation_errors {
            error.validate()?;
            if fields.contains(&error.field.as_str()) {
                return Err(Error::validation(
                    "interactive.action.parameters.validation_errors.field",
                    "must name each field at most once",
                ));
            }
            fields.push(&error.field);
        }
        Ok(())
    }

    /// Renders the `interactive` object for a validated message.
    pub(crate) fn render(&self) -> Value {
        let mut parameters = Map::new();
        parameters.insert("country".into(), Value::String(self.country.clone()));
        if !self.values.is_empty() {
            parameters.insert("values".into(), self.values.to_json());
        }
        if !self.saved_addresses.is_empty() {
            parameters.insert(
                "saved_addresses".into(),
                Value::Array(
                    self.saved_addresses
                        .iter()
                        .map(SavedAddress::to_json)
                        .collect(),
                ),
            );
        }
        if !self.validation_errors.is_empty() {
            let mut errors = Map::new();
            for error in &self.validation_errors {
                errors.insert(error.field.clone(), Value::String(error.message.clone()));
            }
            parameters.insert("validation_errors".into(), Value::Object(errors));
        }

        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("address_message".into()));
        if let Some(header) = &self.header {
            interactive.insert(
                "header".into(),
                json!({ "type": "text", "text": header.text }),
            );
        }
        interactive.insert("body".into(), json!({ "text": self.body }));
        if let Some(footer) = &self.footer {
            interactive.insert("footer".into(), json!({ "text": footer }));
        }
        interactive.insert(
            "action".into(),
            json!({ "name": "address_message", "parameters": Value::Object(parameters) }),
        );
        Value::Object(interactive)
    }
}

/// What the customer submitted: the address they typed, or the id of the saved
/// one they picked.
///
/// Meta delivers it as `interactive.nfm_reply.response_json`, a JSON *string*;
/// [`AddressReply::from_message`] finds it on an inbound [`Message`] and
/// [`AddressReply::parse`] decodes it.
///
/// ```
/// use meta_whatsapp::AddressReply;
///
/// let reply = AddressReply::parse(r#"{"values":{"name":"Ada","city":"Pune"}}"#)?;
/// assert_eq!(reply.values.city.as_deref(), Some("Pune"));
/// assert_eq!(reply.saved_address_id, None);
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AddressReply {
    /// Id of the saved address the customer picked, when they picked one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_address_id: Option<String>,
    /// The submitted fields.
    #[serde(default)]
    pub values: AddressValues,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl AddressReply {
    /// Decodes the `response_json` string of an `nfm_reply`.
    ///
    /// ```
    /// use meta_whatsapp::AddressReply;
    ///
    /// let reply = AddressReply::parse(
    ///     r#"{"saved_address_id":"address1","values":{"in_pin_code":"400063"}}"#,
    /// )?;
    /// assert_eq!(reply.saved_address_id.as_deref(), Some("address1"));
    /// assert_eq!(reply.values.get("in_pin_code"), Some("400063"));
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Json`] when the string is not JSON of this shape.
    pub fn parse(response_json: &str) -> Result<Self> {
        Ok(serde_json::from_str(response_json)?)
    }

    /// Decodes an `interactive.nfm_reply` object.
    ///
    /// The object's `name`, `body` and any other members stay where they are —
    /// only `response_json` is decoded here.
    ///
    /// ```
    /// use meta_whatsapp::types::FlowReply;
    /// use meta_whatsapp::AddressReply;
    ///
    /// let nfm: FlowReply = serde_json::from_value(serde_json::json!({
    ///     "name": "address_message",
    ///     "body": "Ada",
    ///     "response_json": "{\"values\":{\"city\":\"Pune\"}}"
    /// }))?;
    /// let reply = AddressReply::from_nfm_reply(&nfm)?;
    /// assert_eq!(reply.values.city.as_deref(), Some("Pune"));
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when `name` is another flow, or when `response_json`
    /// is missing; [`Error::Json`] when that string does not parse.
    pub fn from_nfm_reply(nfm_reply: &FlowReply) -> Result<Self> {
        match nfm_reply.name.as_deref() {
            None | Some("address_message") => {}
            Some(other) => {
                return Err(Error::Payload(format!(
                    "interactive.nfm_reply.name is {other:?}, not \"address_message\""
                )));
            }
        }
        let Some(response_json) = nfm_reply.response_json.as_deref() else {
            return Err(Error::Payload(
                "interactive.nfm_reply.response_json is missing or not a string".to_string(),
            ));
        };
        Self::parse(response_json)
    }

    /// Finds and decodes the address reply on an inbound message.
    ///
    /// `None` means the message is not an address reply (no `nfm_reply`, or an
    /// `nfm_reply` with another `name`); `Some(Err(_))` means it is one, but the
    /// payload inside does not parse.
    ///
    /// ```
    /// use meta_whatsapp::{AddressReply, Message};
    ///
    /// let message: Message = serde_json::from_str(
    ///     r#"{"id":"wamid.1","from":"4915","type":"interactive",
    ///         "interactive":{"type":"nfm_reply","nfm_reply":{
    ///             "name":"address_message",
    ///             "response_json":"{\"values\":{\"city\":\"Pune\"}}"}}}"#,
    /// ).unwrap();
    /// let reply = AddressReply::from_message(&message).transpose()?.unwrap();
    /// assert_eq!(reply.values.city.as_deref(), Some("Pune"));
    ///
    /// let text: Message = serde_json::from_str(
    ///     r#"{"id":"wamid.2","from":"4915","type":"text","text":{"body":"hi"}}"#,
    /// ).unwrap();
    /// assert!(AddressReply::from_message(&text).is_none());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn from_message(message: &Message) -> Option<Result<Self>> {
        let nfm_reply = message.interactive.as_ref()?.nfm_reply.as_ref()?;
        if nfm_reply.name.as_deref() != Some("address_message") {
            return None;
        }
        Some(Self::from_nfm_reply(nfm_reply))
    }
}

/// `true` when `value` is an optional `+` followed by 7–15 digits.
fn require_phone_number(field: &'static str, value: &str) -> Result<()> {
    let digits = match value.strip_prefix('+') {
        Some(rest) => rest,
        None => value,
    };
    if digits.len() < PHONE_MIN_DIGITS
        || digits.len() > PHONE_MAX_DIGITS
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(Error::validation(
            field,
            format!(
                "must be a phone number: an optional '+', then {PHONE_MIN_DIGITS}..={PHONE_MAX_DIGITS} digits"
            ),
        ));
    }
    Ok(())
}

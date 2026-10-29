//! Commerce messages: catalogs, products, orders and delivery addresses.
//!
//! Catalogs and products are interactive messages on the wire. The outbound
//! types in this module all implement `Into<MessageBody>`, so they can be sent
//! through [`crate::WhatsAppClient::send`] without a wrapper. Inbound orders
//! and address replies are available through [`crate::Message::order`] and
//! [`crate::Message::address`].
//!
//! ```
//! use meta_whatsapp::{CatalogMessage, MessageRequest, ProductMessage};
//!
//! let catalog = MessageRequest::new("4915112345678", CatalogMessage::new("sku-thumb"))
//!     .to_json()?;
//! assert_eq!(catalog["type"], "interactive");
//! assert_eq!(catalog["interactive"]["type"], "catalog_message");
//! assert_eq!(
//!     catalog["interactive"]["action"]["parameters"]["thumbnail_product_retailer_id"],
//!     "sku-thumb",
//! );
//!
//! let product = MessageRequest::new(
//!     "4915112345678",
//!     ProductMessage::new("catalog-1", "sku-1").body("Our best coffee"),
//! )
//! .to_json()?;
//! assert_eq!(product["interactive"]["type"], "product");
//! # Ok::<(), meta_whatsapp::Error>(())
//! ```

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::message::{
    require_max_bytes, require_max_chars, require_no_controls, require_non_empty, HeaderText,
    CAPTION_MAX_BYTES, HEADER_MAX_CHARS,
};

/// Maximum length of a Meta catalog id, in characters.
///
/// ```
/// use meta_whatsapp::commerce::CATALOG_ID_MAX_CHARS;
/// assert_eq!(CATALOG_ID_MAX_CHARS, 100);
/// ```
pub const CATALOG_ID_MAX_CHARS: usize = 100;

/// Maximum length of a catalog product retailer id, in characters.
///
/// ```
/// use meta_whatsapp::commerce::PRODUCT_RETAILER_ID_MAX_CHARS;
/// assert_eq!(PRODUCT_RETAILER_ID_MAX_CHARS, 100);
/// ```
pub const PRODUCT_RETAILER_ID_MAX_CHARS: usize = 100;

/// Maximum number of products in one multi-product message, in total.
///
/// ```
/// use meta_whatsapp::commerce::PRODUCTS_MAX;
/// assert_eq!(PRODUCTS_MAX, 30);
/// ```
pub const PRODUCTS_MAX: usize = 30;

/// Maximum number of sections in one multi-product message.
///
/// ```
/// use meta_whatsapp::commerce::PRODUCT_SECTIONS_MAX;
/// assert_eq!(PRODUCT_SECTIONS_MAX, 10);
/// ```
pub const PRODUCT_SECTIONS_MAX: usize = 10;

/// Maximum length of a product-section title, in characters.
///
/// ```
/// use meta_whatsapp::commerce::PRODUCT_SECTION_TITLE_MAX_CHARS;
/// assert_eq!(PRODUCT_SECTION_TITLE_MAX_CHARS, 24);
/// ```
pub const PRODUCT_SECTION_TITLE_MAX_CHARS: usize = 24;

/// Maximum body length for commerce messages, in UTF-8 bytes.
///
/// ```
/// use meta_whatsapp::commerce::COMMERCE_BODY_MAX_BYTES;
/// assert_eq!(COMMERCE_BODY_MAX_BYTES, 1024);
/// ```
pub const COMMERCE_BODY_MAX_BYTES: usize = CAPTION_MAX_BYTES;

/// Maximum footer length for commerce messages, in characters.
///
/// ```
/// use meta_whatsapp::commerce::COMMERCE_FOOTER_MAX_CHARS;
/// assert_eq!(COMMERCE_FOOTER_MAX_CHARS, 60);
/// ```
pub const COMMERCE_FOOTER_MAX_CHARS: usize = HEADER_MAX_CHARS;

/// Maximum number of line items in one order.
///
/// ```
/// use meta_whatsapp::commerce::ORDER_ITEMS_MAX;
/// assert_eq!(ORDER_ITEMS_MAX, 30);
/// ```
pub const ORDER_ITEMS_MAX: usize = 30;

/// Maximum length of an order reference id, in characters.
///
/// ```
/// use meta_whatsapp::commerce::ORDER_REFERENCE_ID_MAX_CHARS;
/// assert_eq!(ORDER_REFERENCE_ID_MAX_CHARS, 35);
/// ```
pub const ORDER_REFERENCE_ID_MAX_CHARS: usize = 35;

/// Maximum length of an order summary, in UTF-8 bytes.
///
/// ```
/// use meta_whatsapp::commerce::ORDER_TEXT_MAX_BYTES;
/// assert_eq!(ORDER_TEXT_MAX_BYTES, 1024);
/// ```
pub const ORDER_TEXT_MAX_BYTES: usize = CAPTION_MAX_BYTES;

/// Maximum length of an order-status description, in UTF-8 bytes.
///
/// ```
/// use meta_whatsapp::commerce::ORDER_DESCRIPTION_MAX_BYTES;
/// assert_eq!(ORDER_DESCRIPTION_MAX_BYTES, 1024);
/// ```
pub const ORDER_DESCRIPTION_MAX_BYTES: usize = CAPTION_MAX_BYTES;

/// Maximum number of pre-filled values in an address message.
///
/// ```
/// use meta_whatsapp::commerce::ADDRESS_VALUES_MAX;
/// assert_eq!(ADDRESS_VALUES_MAX, 20);
/// ```
pub const ADDRESS_VALUES_MAX: usize = 20;

/// Maximum number of saved addresses offered in an address message.
///
/// ```
/// use meta_whatsapp::commerce::ADDRESS_SAVED_MAX;
/// assert_eq!(ADDRESS_SAVED_MAX, 10);
/// ```
pub const ADDRESS_SAVED_MAX: usize = 10;

/// Maximum length of an address field name, in characters.
///
/// ```
/// use meta_whatsapp::commerce::ADDRESS_FIELD_NAME_MAX_CHARS;
/// assert_eq!(ADDRESS_FIELD_NAME_MAX_CHARS, 64);
/// ```
pub const ADDRESS_FIELD_NAME_MAX_CHARS: usize = 64;

/// Maximum length of an address field value, in UTF-8 bytes.
///
/// ```
/// use meta_whatsapp::commerce::ADDRESS_FIELD_VALUE_MAX_BYTES;
/// assert_eq!(ADDRESS_FIELD_VALUE_MAX_BYTES, 1024);
/// ```
pub const ADDRESS_FIELD_VALUE_MAX_BYTES: usize = CAPTION_MAX_BYTES;

/// Maximum length of a saved-address id, in characters.
///
/// ```
/// use meta_whatsapp::commerce::ADDRESS_ID_MAX_CHARS;
/// assert_eq!(ADDRESS_ID_MAX_CHARS, 100);
/// ```
pub const ADDRESS_ID_MAX_CHARS: usize = 100;

/// An interactive catalog message opened from a product thumbnail.
///
/// ```
/// use meta_whatsapp::{CatalogMessage, MessageRequest};
///
/// let json = MessageRequest::new("1", CatalogMessage::new("sku-1")).to_json()?;
/// assert_eq!(json["type"], "interactive");
/// assert_eq!(json["interactive"]["type"], "catalog_message");
/// assert_eq!(json["interactive"]["action"]["name"], "catalog_message");
/// assert_eq!(
///     json["interactive"]["action"]["parameters"]["thumbnail_product_retailer_id"],
///     "sku-1",
/// );
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogMessage {
    /// Body shown above the catalog button, at most 1024 bytes.
    pub body: String,
    /// Optional footer, at most 60 characters.
    pub footer: Option<String>,
    /// Retailer id of the product shown as the catalog thumbnail, at most 100
    /// characters.
    pub thumbnail_product_retailer_id: String,
}

impl CatalogMessage {
    /// A catalog message whose thumbnail is `thumbnail_product_retailer_id`.
    ///
    /// ```
    /// use meta_whatsapp::CatalogMessage;
    /// assert_eq!(CatalogMessage::new("sku-1").thumbnail_product_retailer_id, "sku-1");
    /// ```
    pub fn new(thumbnail_product_retailer_id: impl Into<String>) -> Self {
        Self {
            body: "Browse our catalog.".to_string(),
            footer: None,
            thumbnail_product_retailer_id: thumbnail_product_retailer_id.into(),
        }
    }

    /// Overrides the body text.
    ///
    /// ```
    /// use meta_whatsapp::CatalogMessage;
    /// assert_eq!(CatalogMessage::new("sku-1").body("Our shop").body, "Our shop");
    /// ```
    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Adds a footer.
    ///
    /// ```
    /// use meta_whatsapp::CatalogMessage;
    /// assert!(CatalogMessage::new("sku-1").footer("Prices include VAT").footer.is_some());
    /// ```
    #[must_use]
    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Checks the thumbnail id, body and footer.
    ///
    /// ```
    /// use meta_whatsapp::CatalogMessage;
    ///
    /// assert!(CatalogMessage::new("sku-1").validate().is_ok());
    /// assert!(CatalogMessage::new("").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the first invalid field.
    pub fn validate(&self) -> Result<()> {
        validate_product_id(
            "interactive.action.parameters.thumbnail_product_retailer_id",
            &self.thumbnail_product_retailer_id,
        )?;
        validate_body("interactive.body.text", &self.body)?;
        validate_footer(self.footer.as_deref())
    }

    pub(crate) fn render(&self) -> Value {
        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("catalog_message".into()));
        interactive.insert("body".into(), json!({ "text": self.body }));
        if let Some(footer) = &self.footer {
            interactive.insert("footer".into(), json!({ "text": footer }));
        }
        interactive.insert(
            "action".into(),
            json!({
                "name": "catalog_message",
                "parameters": {
                    "thumbnail_product_retailer_id": self.thumbnail_product_retailer_id,
                },
            }),
        );
        Value::Object(interactive)
    }
}

/// A single-product interactive message.
///
/// ```
/// use meta_whatsapp::{MessageRequest, ProductMessage};
///
/// let json = MessageRequest::new(
///     "1",
///     ProductMessage::new("catalog-1", "sku-1")
///         .body("Single origin")
///         .footer("Ships tomorrow"),
/// )
/// .to_json()?;
/// assert_eq!(json["interactive"]["type"], "product");
/// assert_eq!(json["interactive"]["action"]["product_retailer_id"], "sku-1");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductMessage {
    /// Body text, at most 1024 bytes.
    pub body: String,
    /// Optional footer, at most 60 characters.
    pub footer: Option<String>,
    /// Catalog that owns the product, at most 100 characters.
    pub catalog_id: String,
    /// Product shown in the message, at most 100 characters.
    pub product_retailer_id: String,
}

impl ProductMessage {
    /// A product message with a generic prompt.
    ///
    /// ```
    /// use meta_whatsapp::ProductMessage;
    /// assert_eq!(ProductMessage::new("c", "p").catalog_id, "c");
    /// ```
    pub fn new(catalog_id: impl Into<String>, product_retailer_id: impl Into<String>) -> Self {
        Self {
            body: "Tap to view this product.".to_string(),
            footer: None,
            catalog_id: catalog_id.into(),
            product_retailer_id: product_retailer_id.into(),
        }
    }

    /// Overrides the body text.
    ///
    /// ```
    /// use meta_whatsapp::ProductMessage;
    /// assert_eq!(ProductMessage::new("c", "p").body("Coffee").body, "Coffee");
    /// ```
    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Adds a footer.
    ///
    /// ```
    /// use meta_whatsapp::ProductMessage;
    /// assert!(ProductMessage::new("c", "p").footer("Details").footer.is_some());
    /// ```
    #[must_use]
    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Checks ids, body and footer.
    ///
    /// ```
    /// use meta_whatsapp::ProductMessage;
    ///
    /// assert!(ProductMessage::new("c", "p").validate().is_ok());
    /// assert!(ProductMessage::new("", "p").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or oversized field.
    pub fn validate(&self) -> Result<()> {
        validate_catalog_id("interactive.action.catalog_id", &self.catalog_id)?;
        validate_product_id(
            "interactive.action.product_retailer_id",
            &self.product_retailer_id,
        )?;
        validate_body("interactive.body.text", &self.body)?;
        validate_footer(self.footer.as_deref())
    }

    pub(crate) fn render(&self) -> Value {
        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("product".into()));
        interactive.insert("body".into(), json!({ "text": self.body }));
        if let Some(footer) = &self.footer {
            interactive.insert("footer".into(), json!({ "text": footer }));
        }
        interactive.insert(
            "action".into(),
            json!({
                "catalog_id": self.catalog_id,
                "product_retailer_id": self.product_retailer_id,
            }),
        );
        Value::Object(interactive)
    }
}

/// One product reference in a [`ProductListMessage`] section.
///
/// ```
/// use meta_whatsapp::ProductItem;
/// assert_eq!(ProductItem::new("sku-1").product_retailer_id, "sku-1");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductItem {
    /// Retailer id of the product, at most 100 characters.
    pub product_retailer_id: String,
}

impl ProductItem {
    /// References `product_retailer_id`.
    ///
    /// ```
    /// use meta_whatsapp::ProductItem;
    /// assert_eq!(ProductItem::new("sku-1").product_retailer_id, "sku-1");
    /// ```
    pub fn new(product_retailer_id: impl Into<String>) -> Self {
        Self {
            product_retailer_id: product_retailer_id.into(),
        }
    }

    /// Checks the product retailer id.
    ///
    /// ```
    /// use meta_whatsapp::ProductItem;
    ///
    /// assert!(ProductItem::new("sku-1").validate().is_ok());
    /// assert!(ProductItem::new("").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty, oversized, or control-containing
    /// retailer id.
    pub fn validate(&self) -> Result<()> {
        validate_product_id(
            "interactive.action.sections.product_items.product_retailer_id",
            &self.product_retailer_id,
        )
    }

    fn render(&self) -> Value {
        json!({ "product_retailer_id": self.product_retailer_id })
    }
}

/// A titled group of products in a multi-product message.
///
/// ```
/// use meta_whatsapp::{ProductItem, ProductSection};
///
/// let section = ProductSection::new(vec![ProductItem::new("sku-1")]).title("Coffee");
/// assert_eq!(section.title.as_deref(), Some("Coffee"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductSection {
    /// Optional section title, at most 24 characters. Required when the
    /// message has more than one section.
    pub title: Option<String>,
    /// Products in this section. A message may contain at most 30 in total.
    pub product_items: Vec<ProductItem>,
}

impl ProductSection {
    /// A section with no title.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductSection};
    /// assert!(ProductSection::new(vec![ProductItem::new("sku-1")]).title.is_none());
    /// ```
    #[must_use]
    pub fn new(product_items: Vec<ProductItem>) -> Self {
        Self {
            title: None,
            product_items,
        }
    }

    /// Sets the section title.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductSection};
    /// assert_eq!(
    ///     ProductSection::new(vec![ProductItem::new("sku-1")]).title("Coffee").title.as_deref(),
    ///     Some("Coffee"),
    /// );
    /// ```
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Checks the section title, product count and every product id.
    ///
    /// A [`ProductSection`] used in a message with more than one section must
    /// also have a title; [`ProductListMessage::validate`] enforces that
    /// context-dependent rule.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductSection};
    ///
    /// let section = ProductSection::new(vec![ProductItem::new("sku-1")]);
    /// assert!(section.validate().is_ok());
    /// assert!(ProductSection::new(Vec::new()).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or oversized title, no products, too
    /// many products, or an invalid product id.
    pub fn validate(&self) -> Result<()> {
        self.validate_with(false)
    }

    fn validate_with(&self, title_required: bool) -> Result<()> {
        match &self.title {
            Some(title) => {
                require_non_empty("interactive.action.sections.title", title)?;
                require_max_chars(
                    "interactive.action.sections.title",
                    title,
                    PRODUCT_SECTION_TITLE_MAX_CHARS,
                )?;
            }
            None if title_required => {
                return Err(Error::validation(
                    "interactive.action.sections.title",
                    "must be set when there is more than one section",
                ));
            }
            None => {}
        }
        if self.product_items.is_empty() {
            return Err(Error::validation(
                "interactive.action.sections.product_items",
                "a section needs at least one product",
            ));
        }
        if self.product_items.len() > PRODUCTS_MAX {
            return Err(Error::validation(
                "interactive.action.sections.product_items",
                format!(
                    "must contain at most {PRODUCTS_MAX} products in this section, got {}",
                    self.product_items.len()
                ),
            ));
        }
        for item in &self.product_items {
            item.validate()?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut object = Map::new();
        if let Some(title) = &self.title {
            object.insert("title".into(), Value::String(title.clone()));
        }
        object.insert(
            "product_items".into(),
            Value::Array(self.product_items.iter().map(ProductItem::render).collect()),
        );
        Value::Object(object)
    }
}

/// A multi-product interactive message.
///
/// ```
/// use meta_whatsapp::{MessageRequest, ProductItem, ProductListMessage, ProductSection};
///
/// let json = MessageRequest::new(
///     "1",
///     ProductListMessage::new(
///         "catalog-1",
///         vec![ProductSection::new(vec![ProductItem::new("sku-1")])],
///     ),
/// )
/// .to_json()?;
/// assert_eq!(json["interactive"]["type"], "product_list");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductListMessage {
    /// Text header, at most 60 characters. Meta requires it for this type;
    /// the constructor supplies a generic one.
    pub header: Option<HeaderText>,
    /// Body text, at most 1024 bytes.
    pub body: String,
    /// Optional footer, at most 60 characters.
    pub footer: Option<String>,
    /// Catalog that owns every product in the message, at most 100 characters.
    pub catalog_id: String,
    /// One to ten product sections.
    pub sections: Vec<ProductSection>,
}

impl ProductListMessage {
    /// A multi-product message with generic header and body text.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductListMessage, ProductSection};
    /// let sections = vec![ProductSection::new(vec![ProductItem::new("sku-1")])];
    /// assert_eq!(ProductListMessage::new("c", sections).catalog_id, "c");
    /// ```
    pub fn new(catalog_id: impl Into<String>, sections: Vec<ProductSection>) -> Self {
        Self {
            header: Some(HeaderText::new("Products")),
            body: "Browse our products.".to_string(),
            footer: None,
            catalog_id: catalog_id.into(),
            sections,
        }
    }

    /// Overrides the body text.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductListMessage, ProductSection};
    /// let sections = vec![ProductSection::new(vec![ProductItem::new("sku-1")])];
    /// assert_eq!(ProductListMessage::new("c", sections).body("Choose one").body, "Choose one");
    /// ```
    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Sets the header text.
    ///
    /// ```
    /// use meta_whatsapp::{HeaderText, ProductItem, ProductListMessage, ProductSection};
    /// let sections = vec![ProductSection::new(vec![ProductItem::new("sku-1")])];
    /// assert_eq!(
    ///     ProductListMessage::new("c", sections)
    ///         .header(HeaderText::new("Menu"))
    ///         .header
    ///         .unwrap()
    ///         .text,
    ///     "Menu",
    /// );
    /// ```
    #[must_use]
    pub fn header(mut self, header: HeaderText) -> Self {
        self.header = Some(header);
        self
    }

    /// Adds a footer.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductListMessage, ProductSection};
    /// let sections = vec![ProductSection::new(vec![ProductItem::new("sku-1")])];
    /// assert!(ProductListMessage::new("c", sections).footer("Prices include VAT").footer.is_some());
    /// ```
    #[must_use]
    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Appends a product section.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductListMessage, ProductSection};
    /// let section = ProductSection::new(vec![ProductItem::new("sku-1")]);
    /// assert_eq!(ProductListMessage::new("c", Vec::new()).section(section).sections.len(), 1);
    /// ```
    #[must_use]
    pub fn section(mut self, section: ProductSection) -> Self {
        self.sections.push(section);
        self
    }

    /// Checks sections, products, ids, titles and text limits.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductListMessage, ProductSection};
    /// let sections = vec![ProductSection::new(vec![ProductItem::new("sku-1")])];
    /// assert!(ProductListMessage::new("c", sections).validate().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a missing header, an empty section, too many
    /// sections/products, a missing title on a multi-section message, or an
    /// oversized id/text.
    pub fn validate(&self) -> Result<()> {
        validate_catalog_id("interactive.action.catalog_id", &self.catalog_id)?;
        let Some(header) = &self.header else {
            return Err(Error::validation(
                "interactive.header.text",
                "is required for a product_list message",
            ));
        };
        header.validate()?;
        validate_body("interactive.body.text", &self.body)?;
        validate_footer(self.footer.as_deref())?;
        if self.sections.is_empty() {
            return Err(Error::validation(
                "interactive.action.sections",
                "must contain at least one section",
            ));
        }
        if self.sections.len() > PRODUCT_SECTIONS_MAX {
            return Err(Error::validation(
                "interactive.action.sections",
                format!(
                    "must contain at most {PRODUCT_SECTIONS_MAX} sections, got {}",
                    self.sections.len()
                ),
            ));
        }
        let products: usize = self
            .sections
            .iter()
            .map(|section| section.product_items.len())
            .sum();
        if products > PRODUCTS_MAX {
            return Err(Error::validation(
                "interactive.action.sections.product_items",
                format!("must contain at most {PRODUCTS_MAX} products in total, got {products}"),
            ));
        }
        let title_required = self.sections.len() > 1;
        for section in &self.sections {
            section.validate_with(title_required)?;
        }
        Ok(())
    }

    pub(crate) fn render(&self) -> Value {
        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("product_list".into()));
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
            json!({
                "catalog_id": self.catalog_id,
                "sections": self.sections.iter().map(ProductSection::render).collect::<Vec<_>>(),
            }),
        );
        Value::Object(interactive)
    }
}

fn validate_product_id(field: &'static str, value: &str) -> Result<()> {
    require_non_empty(field, value)?;
    require_max_chars(field, value, PRODUCT_RETAILER_ID_MAX_CHARS)?;
    require_no_controls(field, value)
}

fn validate_catalog_id(field: &'static str, value: &str) -> Result<()> {
    require_non_empty(field, value)?;
    require_max_chars(field, value, CATALOG_ID_MAX_CHARS)?;
    require_no_controls(field, value)
}

fn validate_body(field: &'static str, value: &str) -> Result<()> {
    require_non_empty(field, value)?;
    require_max_bytes(field, value, COMMERCE_BODY_MAX_BYTES)
}

fn validate_footer(footer: Option<&str>) -> Result<()> {
    if let Some(footer) = footer {
        require_max_chars("interactive.footer.text", footer, COMMERCE_FOOTER_MAX_CHARS)?;
    }
    Ok(())
}

/// One product line in an [`Order`].
///
/// The same type parses the `order.product_items` array in a webhook and
/// renders the array in an [`OrderDetailsMessage`]. Inbound numeric strings,
/// camelCase aliases and unknown keys are accepted.
///
/// ```
/// use meta_whatsapp::OrderItem;
///
/// let item = OrderItem::new("sku-1", 2, 9.5, "EUR");
/// assert_eq!(item.quantity, 2);
/// assert_eq!(item.currency, "EUR");
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct OrderItem {
    /// Retailer id of the ordered product.
    #[serde(default, alias = "productRetailerId")]
    pub product_retailer_id: String,
    /// Number of units, at least one.
    #[serde(default, deserialize_with = "deserialize_quantity")]
    pub quantity: u32,
    /// Price per unit in major currency units; finite and non-negative.
    #[serde(default, alias = "itemPrice", deserialize_with = "deserialize_price")]
    pub item_price: f64,
    /// ISO 4217 currency code, three uppercase ASCII letters.
    #[serde(default)]
    pub currency: String,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl OrderItem {
    /// A line item with all fields Meta needs for an order.
    ///
    /// ```
    /// use meta_whatsapp::OrderItem;
    /// assert_eq!(OrderItem::new("sku-1", 1, 4.0, "USD").item_price, 4.0);
    /// ```
    pub fn new(
        product_retailer_id: impl Into<String>,
        quantity: u32,
        item_price: f64,
        currency: impl Into<String>,
    ) -> Self {
        Self {
            product_retailer_id: product_retailer_id.into(),
            quantity,
            item_price,
            currency: currency.into(),
            extra: Map::new(),
        }
    }

    /// Checks the product id, quantity, price and currency.
    ///
    /// ```
    /// use meta_whatsapp::OrderItem;
    ///
    /// assert!(OrderItem::new("sku-1", 1, 0.0, "EUR").validate().is_ok());
    /// assert!(OrderItem::new("sku-1", 0, 1.0, "EUR").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the first invalid field.
    pub fn validate(&self) -> Result<()> {
        validate_product_id(
            "interactive.action.parameters.order.product_items.product_retailer_id",
            &self.product_retailer_id,
        )?;
        if self.quantity == 0 {
            return Err(Error::validation(
                "interactive.action.parameters.order.product_items.quantity",
                "must be at least 1",
            ));
        }
        if !self.item_price.is_finite() || self.item_price < 0.0 {
            return Err(Error::validation(
                "interactive.action.parameters.order.product_items.item_price",
                "must be finite and at least 0",
            ));
        }
        validate_currency(
            "interactive.action.parameters.order.product_items.currency",
            &self.currency,
        )
    }

    fn render(&self) -> Value {
        json!({
            "product_retailer_id": self.product_retailer_id,
            "quantity": self.quantity,
            "item_price": self.item_price,
            "currency": self.currency,
        })
    }
}

/// The order object shared by an order-details message and an inbound order.
///
/// ```
/// use meta_whatsapp::{Order, OrderItem};
///
/// let order = Order::new("catalog-1", vec![OrderItem::new("sku-1", 1, 4.0, "EUR")])
///     .text("One espresso");
/// assert_eq!(order.product_items.len(), 1);
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Order {
    /// Catalog that owns the products.
    #[serde(default, alias = "catalogId")]
    pub catalog_id: String,
    /// Optional summary shown to the customer (or the empty string Meta sends
    /// on an inbound order).
    #[serde(default)]
    pub text: Option<String>,
    /// Ordered products.
    #[serde(default, alias = "productItems")]
    pub product_items: Vec<OrderItem>,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Order {
    /// An order for `product_items` from `catalog_id`.
    ///
    /// ```
    /// use meta_whatsapp::{Order, OrderItem};
    /// assert_eq!(Order::new("c", vec![OrderItem::new("p", 1, 1.0, "USD")]).catalog_id, "c");
    /// ```
    pub fn new(catalog_id: impl Into<String>, product_items: Vec<OrderItem>) -> Self {
        Self {
            catalog_id: catalog_id.into(),
            text: None,
            product_items,
            extra: Map::new(),
        }
    }

    /// Sets the order summary.
    ///
    /// ```
    /// use meta_whatsapp::{Order, OrderItem};
    /// let order = Order::new("c", vec![OrderItem::new("p", 1, 1.0, "USD")]).text("Total: 1 USD");
    /// assert_eq!(order.text.as_deref(), Some("Total: 1 USD"));
    /// ```
    #[must_use]
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Appends a line item.
    ///
    /// ```
    /// use meta_whatsapp::{Order, OrderItem};
    /// let order = Order::new("c", Vec::new()).item(OrderItem::new("p", 1, 1.0, "USD"));
    /// assert_eq!(order.product_items.len(), 1);
    /// ```
    #[must_use]
    pub fn item(mut self, item: OrderItem) -> Self {
        self.product_items.push(item);
        self
    }

    /// Checks the catalog, summary and every line item.
    ///
    /// ```
    /// use meta_whatsapp::{Order, OrderItem};
    ///
    /// let order = Order::new("c", vec![OrderItem::new("p", 1, 1.0, "USD")]);
    /// assert!(order.validate().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty order, too many items, or an invalid
    /// line item.
    pub fn validate(&self) -> Result<()> {
        validate_catalog_id(
            "interactive.action.parameters.order.catalog_id",
            &self.catalog_id,
        )?;
        if let Some(text) = &self.text {
            require_max_bytes(
                "interactive.action.parameters.order.text",
                text,
                ORDER_TEXT_MAX_BYTES,
            )?;
        }
        if self.product_items.is_empty() {
            return Err(Error::validation(
                "interactive.action.parameters.order.product_items",
                "must contain at least one item",
            ));
        }
        if self.product_items.len() > ORDER_ITEMS_MAX {
            return Err(Error::validation(
                "interactive.action.parameters.order.product_items",
                format!(
                    "must contain at most {ORDER_ITEMS_MAX} items, got {}",
                    self.product_items.len()
                ),
            ));
        }
        for item in &self.product_items {
            item.validate()?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut object = Map::new();
        object.insert("catalog_id".into(), Value::String(self.catalog_id.clone()));
        if let Some(text) = &self.text {
            object.insert("text".into(), Value::String(text.clone()));
        }
        object.insert(
            "product_items".into(),
            Value::Array(self.product_items.iter().map(OrderItem::render).collect()),
        );
        Value::Object(object)
    }
}

/// An interactive order-details message.
///
/// ```
/// use meta_whatsapp::{MessageRequest, Order, OrderDetailsMessage, OrderItem};
///
/// let order = Order::new("catalog-1", vec![OrderItem::new("sku-1", 1, 4.0, "EUR")]);
/// let json = MessageRequest::new("1", OrderDetailsMessage::new("order-42", order)).to_json()?;
/// assert_eq!(json["interactive"]["type"], "order_details");
/// assert_eq!(json["interactive"]["action"]["name"], "review_order");
/// assert_eq!(json["interactive"]["action"]["parameters"]["reference_id"], "order-42");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct OrderDetailsMessage {
    /// Body text, at most 1024 bytes.
    pub body: String,
    /// Optional footer, at most 60 characters.
    pub footer: Option<String>,
    /// Your order id, echoed back in order-status updates.
    pub reference_id: String,
    /// Catalog, summary and line items.
    pub order: Order,
}

impl OrderDetailsMessage {
    /// An order-details message for `reference_id`.
    ///
    /// ```
    /// use meta_whatsapp::{Order, OrderDetailsMessage, OrderItem};
    /// let order = Order::new("c", vec![OrderItem::new("p", 1, 1.0, "USD")]);
    /// assert_eq!(OrderDetailsMessage::new("ref", order).reference_id, "ref");
    /// ```
    pub fn new(reference_id: impl Into<String>, order: Order) -> Self {
        Self {
            body: "Here are your order details.".to_string(),
            footer: None,
            reference_id: reference_id.into(),
            order,
        }
    }

    /// Overrides the body text.
    ///
    /// ```
    /// use meta_whatsapp::{Order, OrderDetailsMessage, OrderItem};
    /// let order = Order::new("c", vec![OrderItem::new("p", 1, 1.0, "USD")]);
    /// assert_eq!(OrderDetailsMessage::new("r", order).body("Thanks").body, "Thanks");
    /// ```
    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Adds a footer.
    ///
    /// ```
    /// use meta_whatsapp::{Order, OrderDetailsMessage, OrderItem};
    /// let order = Order::new("c", vec![OrderItem::new("p", 1, 1.0, "USD")]);
    /// assert!(OrderDetailsMessage::new("r", order).footer("Review before paying").footer.is_some());
    /// ```
    #[must_use]
    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Checks the body, footer, reference id and order.
    ///
    /// ```
    /// use meta_whatsapp::{Order, OrderDetailsMessage, OrderItem};
    /// let order = Order::new("c", vec![OrderItem::new("p", 1, 1.0, "USD")]);
    /// assert!(OrderDetailsMessage::new("r", order).validate().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or oversized field, or an invalid
    /// order.
    pub fn validate(&self) -> Result<()> {
        validate_body("interactive.body.text", &self.body)?;
        validate_footer(self.footer.as_deref())?;
        validate_reference_id(&self.reference_id)?;
        self.order.validate()
    }

    pub(crate) fn render(&self) -> Value {
        let mut parameters = Map::new();
        parameters.insert(
            "reference_id".into(),
            Value::String(self.reference_id.clone()),
        );
        parameters.insert("order".into(), self.order.render());

        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("order_details".into()));
        interactive.insert("body".into(), json!({ "text": self.body }));
        if let Some(footer) = &self.footer {
            interactive.insert("footer".into(), json!({ "text": footer }));
        }
        interactive.insert(
            "action".into(),
            json!({ "name": "review_order", "parameters": Value::Object(parameters) }),
        );
        Value::Object(interactive)
    }
}

/// Convenience alias for [`OrderDetailsMessage`].
///
/// ```
/// use meta_whatsapp::{Order, OrderItem, OrderMessage};
/// let order = Order::new("c", vec![OrderItem::new("p", 1, 1.0, "USD")]);
/// assert_eq!(OrderMessage::new("r", order).reference_id, "r");
/// ```
pub type OrderMessage = OrderDetailsMessage;

/// Status of an order update.
///
/// Unknown values are carried instead of rejected, so a status Meta adds later
/// can still be forwarded without a breaking release.
///
/// ```
/// use meta_whatsapp::OrderStatus;
/// assert_eq!(OrderStatus::Shipped.as_str(), "shipped");
/// assert_eq!(OrderStatus::from_wire("new_status").as_str(), "new_status");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum OrderStatus {
    /// Received but not yet processed.
    #[default]
    Pending,
    /// Being prepared.
    Processing,
    /// Some line items have shipped.
    PartiallyShipped,
    /// Handed to the carrier.
    Shipped,
    /// Delivered.
    Completed,
    /// Cancelled (Meta's wire spelling is `canceled`).
    Canceled,
    /// Cancelled, accepted as an alternate spelling.
    Cancelled,
    /// A value this crate does not know yet.
    Unknown(String),
}

impl OrderStatus {
    /// Wire representation.
    ///
    /// ```
    /// use meta_whatsapp::OrderStatus;
    /// assert_eq!(OrderStatus::PartiallyShipped.as_str(), "partially_shipped");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::Processing => "processing",
            Self::PartiallyShipped => "partially_shipped",
            Self::Shipped => "shipped",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
            Self::Cancelled => "cancelled",
            Self::Unknown(value) => value,
        }
    }

    /// Carries an arbitrary wire value, mapping known spellings to variants.
    ///
    /// ```
    /// use meta_whatsapp::OrderStatus;
    /// assert_eq!(OrderStatus::from_wire("processing"), OrderStatus::Processing);
    /// ```
    #[must_use]
    pub fn from_wire(value: impl Into<String>) -> Self {
        let value = value.into();
        match value.as_str() {
            "pending" => Self::Pending,
            "processing" => Self::Processing,
            "partially_shipped" | "partiallyShipped" => Self::PartiallyShipped,
            "shipped" => Self::Shipped,
            "completed" => Self::Completed,
            "canceled" => Self::Canceled,
            "cancelled" => Self::Cancelled,
            _ => Self::Unknown(value),
        }
    }
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for OrderStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for OrderStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Ok(Self::from_wire(String::deserialize(deserializer)?))
    }
}

/// An interactive order-status update.
///
/// ```
/// use meta_whatsapp::{MessageRequest, OrderStatus, OrderStatusMessage};
///
/// let json = MessageRequest::new(
///     "1",
///     OrderStatusMessage::new("order-42", OrderStatus::Shipped)
///         .description("Arrives tomorrow"),
/// )
/// .to_json()?;
/// assert_eq!(json["interactive"]["type"], "order_status");
/// assert_eq!(json["interactive"]["action"]["name"], "review_order");
/// assert_eq!(json["interactive"]["action"]["parameters"]["order"]["status"], "shipped");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderStatusMessage {
    /// Body text, at most 1024 bytes.
    pub body: String,
    /// The order id sent with the order details.
    pub reference_id: String,
    /// New status.
    pub status: OrderStatus,
    /// Optional explanation, at most 1024 bytes.
    pub description: Option<String>,
}

impl OrderStatusMessage {
    /// A status update with a generic body.
    ///
    /// ```
    /// use meta_whatsapp::{OrderStatus, OrderStatusMessage};
    /// assert_eq!(OrderStatusMessage::new("r", OrderStatus::Processing).status, OrderStatus::Processing);
    /// ```
    pub fn new(reference_id: impl Into<String>, status: OrderStatus) -> Self {
        Self {
            body: "Your order status has changed.".to_string(),
            reference_id: reference_id.into(),
            status,
            description: None,
        }
    }

    /// Overrides the body text.
    ///
    /// ```
    /// use meta_whatsapp::{OrderStatus, OrderStatusMessage};
    /// assert_eq!(OrderStatusMessage::new("r", OrderStatus::Shipped).body("On its way").body, "On its way");
    /// ```
    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Adds an explanation.
    ///
    /// ```
    /// use meta_whatsapp::{OrderStatus, OrderStatusMessage};
    /// assert!(OrderStatusMessage::new("r", OrderStatus::Shipped).description("Tomorrow").description.is_some());
    /// ```
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Checks the body, reference id, description and status.
    ///
    /// ```
    /// use meta_whatsapp::{OrderStatus, OrderStatusMessage};
    /// assert!(OrderStatusMessage::new("r", OrderStatus::Shipped).validate().is_ok());
    /// assert!(OrderStatusMessage::new("r", OrderStatus::Unknown(String::new())).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or oversized field, or an empty
    /// unknown status spelling.
    pub fn validate(&self) -> Result<()> {
        validate_body("interactive.body.text", &self.body)?;
        validate_reference_id(&self.reference_id)?;
        if let Some(description) = &self.description {
            require_max_bytes(
                "interactive.action.parameters.order.description",
                description,
                ORDER_DESCRIPTION_MAX_BYTES,
            )?;
        }
        if self.status.as_str().is_empty() {
            return Err(Error::validation(
                "interactive.action.parameters.order.status",
                "must not be empty",
            ));
        }
        require_no_controls(
            "interactive.action.parameters.order.status",
            self.status.as_str(),
        )
    }

    pub(crate) fn render(&self) -> Value {
        let mut order = Map::new();
        order.insert(
            "status".into(),
            Value::String(self.status.as_str().to_string()),
        );
        if let Some(description) = &self.description {
            order.insert("description".into(), Value::String(description.clone()));
        }

        let mut parameters = Map::new();
        parameters.insert(
            "reference_id".into(),
            Value::String(self.reference_id.clone()),
        );
        parameters.insert("order".into(), Value::Object(order));

        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("order_status".into()));
        interactive.insert("body".into(), json!({ "text": self.body }));
        interactive.insert(
            "action".into(),
            json!({ "name": "review_order", "parameters": Value::Object(parameters) }),
        );
        Value::Object(interactive)
    }
}

fn validate_reference_id(value: &str) -> Result<()> {
    require_non_empty("interactive.action.parameters.reference_id", value)?;
    require_max_chars(
        "interactive.action.parameters.reference_id",
        value,
        ORDER_REFERENCE_ID_MAX_CHARS,
    )?;
    require_no_controls("interactive.action.parameters.reference_id", value)
}

fn validate_currency(field: &'static str, value: &str) -> Result<()> {
    if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(Error::validation(
            field,
            "must be a three-letter uppercase ISO 4217 code",
        ));
    }
    Ok(())
}

fn deserialize_quantity<'de, D>(deserializer: D) -> std::result::Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| D::Error::custom("quantity must be an unsigned 32-bit integer")),
        Value::String(value) => value
            .parse()
            .map_err(|_| D::Error::custom("quantity must be an unsigned 32-bit integer")),
        _ => Err(D::Error::custom(
            "quantity must be an unsigned 32-bit integer",
        )),
    }
}

fn deserialize_price<'de, D>(deserializer: D) -> std::result::Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let price = match Value::deserialize(deserializer)? {
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| D::Error::custom("item_price must be a number")),
        Value::String(value) => value
            .parse()
            .map_err(|_| D::Error::custom("item_price must be a number")),
        _ => Err(D::Error::custom("item_price must be a number")),
    }?;
    if !price.is_finite() {
        return Err(D::Error::custom("item_price must be finite"));
    }
    Ok(price)
}

/// Category of a pre-filled address field.
///
/// ```
/// use meta_whatsapp::AddressFieldType;
/// assert_eq!(AddressFieldType::Address.as_str(), "address");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum AddressFieldType {
    /// Customer or recipient name.
    #[default]
    Name,
    /// Street, city, state, postal code or country line.
    Address,
    /// Phone number.
    Phone,
    /// E-mail address.
    Email,
    /// A category this crate does not know yet.
    Unknown(String),
}

impl AddressFieldType {
    /// Wire representation.
    ///
    /// ```
    /// use meta_whatsapp::AddressFieldType;
    /// assert_eq!(AddressFieldType::Phone.as_str(), "phone");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Name => "name",
            Self::Address => "address",
            Self::Phone => "phone",
            Self::Email => "email",
            Self::Unknown(value) => value,
        }
    }

    /// Carries an arbitrary wire value, mapping known spellings to variants.
    ///
    /// ```
    /// use meta_whatsapp::AddressFieldType;
    /// assert_eq!(AddressFieldType::from_wire("email"), AddressFieldType::Email);
    /// assert_eq!(
    ///     AddressFieldType::from_wire("nickname"),
    ///     AddressFieldType::Unknown("nickname".into()),
    /// );
    /// ```
    #[must_use]
    pub fn from_wire(value: impl Into<String>) -> Self {
        let value = value.into();
        match value.as_str() {
            "name" => Self::Name,
            "address" => Self::Address,
            "phone" => Self::Phone,
            "email" => Self::Email,
            _ => Self::Unknown(value),
        }
    }
}

impl std::fmt::Display for AddressFieldType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for AddressFieldType {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AddressFieldType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Ok(Self::from_wire(String::deserialize(deserializer)?))
    }
}

/// One pre-filled value in an address message or address reply.
///
/// ```
/// use meta_whatsapp::{AddressField, AddressFieldType};
///
/// let field = AddressField::new(AddressFieldType::Address, "city", "Berlin");
/// assert_eq!(field.value, "Berlin");
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AddressField {
    /// Category of the value (`name`, `address`, `phone`, `email`, ...).
    #[serde(rename = "type", default)]
    pub field_type: AddressFieldType,
    /// Field name, for example `name`, `city`, `state`, `zip` or `country`.
    #[serde(default)]
    pub name: String,
    /// Pre-filled or customer-supplied value.
    #[serde(default)]
    pub value: String,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl AddressField {
    /// A pre-filled address value.
    ///
    /// ```
    /// use meta_whatsapp::{AddressField, AddressFieldType};
    /// assert_eq!(AddressField::new(AddressFieldType::Name, "name", "Ada").name, "name");
    /// ```
    pub fn new(
        field_type: AddressFieldType,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            field_type,
            name: name.into(),
            value: value.into(),
            extra: Map::new(),
        }
    }

    /// Checks the type, name and value limits used for outbound messages.
    ///
    /// ```
    /// use meta_whatsapp::{AddressField, AddressFieldType};
    ///
    /// let field = AddressField::new(AddressFieldType::Address, "city", "Berlin");
    /// assert!(field.validate().is_ok());
    /// assert!(AddressField::new(AddressFieldType::Address, "", "Berlin").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty type/name, control characters, or an
    /// oversized name/value.
    pub fn validate(&self) -> Result<()> {
        if self.field_type.as_str().is_empty() {
            return Err(Error::validation(
                "interactive.action.parameters.values.type",
                "must not be empty",
            ));
        }
        require_no_controls(
            "interactive.action.parameters.values.type",
            self.field_type.as_str(),
        )?;
        require_non_empty("interactive.action.parameters.values.name", &self.name)?;
        require_max_chars(
            "interactive.action.parameters.values.name",
            &self.name,
            ADDRESS_FIELD_NAME_MAX_CHARS,
        )?;
        require_no_controls("interactive.action.parameters.values.name", &self.name)?;
        require_max_bytes(
            "interactive.action.parameters.values.value",
            &self.value,
            ADDRESS_FIELD_VALUE_MAX_BYTES,
        )
    }

    fn render(&self) -> Value {
        json!({
            "type": self.field_type.as_str(),
            "name": self.name,
            "value": self.value,
        })
    }
}

/// Convenience alias for [`AddressField`].
///
/// ```
/// use meta_whatsapp::{AddressField, AddressFieldType, AddressValue};
/// let value = AddressValue::new(AddressFieldType::Phone, "phone", "+4915");
/// assert!(matches!(value, AddressField { .. }));
/// ```
pub type AddressValue = AddressField;

/// Convenience alias for [`AddressFieldType`].
///
/// ```
/// use meta_whatsapp::{AddressFieldType, AddressType};
/// assert_eq!(AddressType::Address, AddressFieldType::Address);
/// ```
pub type AddressType = AddressFieldType;

/// A saved address offered to the customer as a shortcut.
///
/// ```
/// use meta_whatsapp::SavedAddress;
///
/// let address = SavedAddress::new("home").field("city", "Berlin");
/// assert_eq!(address.value["city"], "Berlin");
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SavedAddress {
    /// Provider or application id for this saved address.
    #[serde(default)]
    pub id: String,
    /// Address fields as a name-to-value object.
    #[serde(default)]
    pub value: Map<String, Value>,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SavedAddress {
    /// A saved address with `id` and no fields yet.
    ///
    /// ```
    /// use meta_whatsapp::SavedAddress;
    /// assert!(SavedAddress::new("home").value.is_empty());
    /// ```
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            value: Map::new(),
            extra: Map::new(),
        }
    }

    /// Adds one field to the saved address.
    ///
    /// ```
    /// use meta_whatsapp::SavedAddress;
    /// assert_eq!(SavedAddress::new("home").field("zip", "10115").value["zip"], "10115");
    /// ```
    #[must_use]
    pub fn field(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.value.insert(name.into(), Value::String(value.into()));
        self
    }

    /// Checks the saved-address id.
    ///
    /// ```
    /// use meta_whatsapp::SavedAddress;
    /// assert!(SavedAddress::new("home").validate().is_ok());
    /// assert!(SavedAddress::new("").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty, oversized or control-containing id.
    pub fn validate(&self) -> Result<()> {
        require_non_empty("interactive.action.parameters.saved_addresses.id", &self.id)?;
        require_max_chars(
            "interactive.action.parameters.saved_addresses.id",
            &self.id,
            ADDRESS_ID_MAX_CHARS,
        )?;
        require_no_controls("interactive.action.parameters.saved_addresses.id", &self.id)
    }

    fn render(&self) -> Value {
        json!({ "id": self.id, "value": self.value })
    }
}

/// The address block Meta sends back after an address message.
///
/// Unknown values and keys survive, so a new address field does not make the
/// webhook fail to parse.
///
/// ```
/// use meta_whatsapp::AddressReply;
///
/// let reply: AddressReply = serde_json::from_str(
///     r#"{"country":"DE","values":[{"type":"address","name":"city","value":"Berlin"}]}"#,
/// )?;
/// assert_eq!(reply.field("city"), Some("Berlin"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AddressReply {
    /// Country code used for the form.
    #[serde(default)]
    pub country: Option<String>,
    /// Address values supplied by the customer.
    #[serde(default)]
    pub values: Vec<AddressField>,
    /// Saved addresses, when Meta echoes them back.
    #[serde(default, alias = "savedAddresses")]
    pub saved_addresses: Vec<SavedAddress>,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl AddressReply {
    /// Looks up a value by its `name` (`city`, `zip`, `name`, ...).
    ///
    /// ```
    /// use meta_whatsapp::AddressReply;
    ///
    /// let reply: AddressReply = serde_json::from_str(
    ///     r#"{"values":[{"type":"name","name":"name","value":"Ada"}]}"#,
    /// )?;
    /// assert_eq!(reply.field("name"), Some("Ada"));
    /// assert_eq!(reply.field("missing"), None);
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|field| field.name == name)
            .map(|field| field.value.as_str())
    }
}

/// An interactive message that asks the customer for a delivery address.
///
/// ```
/// use meta_whatsapp::{AddressField, AddressFieldType, AddressMessage, MessageRequest};
///
/// let json = MessageRequest::new(
///     "1",
///     AddressMessage::new("DE").value(
///         AddressField::new(AddressFieldType::Address, "city", "Berlin"),
///     ),
/// )
/// .to_json()?;
/// assert_eq!(json["interactive"]["type"], "address_message");
/// assert_eq!(json["interactive"]["action"]["parameters"]["country"], "DE");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressMessage {
    /// Body text, at most 1024 bytes.
    pub body: String,
    /// Two-letter ISO 3166-1 country code.
    pub country: String,
    /// Values used to pre-fill the form.
    pub values: Vec<AddressField>,
    /// Saved addresses offered as shortcuts.
    pub saved_addresses: Vec<SavedAddress>,
}

impl AddressMessage {
    /// An address request for `country` with a generic prompt.
    ///
    /// ```
    /// use meta_whatsapp::AddressMessage;
    /// assert_eq!(AddressMessage::new("DE").country, "DE");
    /// ```
    pub fn new(country: impl Into<String>) -> Self {
        Self {
            body: "Please share your delivery address.".to_string(),
            country: country.into(),
            values: Vec::new(),
            saved_addresses: Vec::new(),
        }
    }

    /// Overrides the body text.
    ///
    /// ```
    /// use meta_whatsapp::AddressMessage;
    /// assert_eq!(AddressMessage::new("DE").body("Where should we deliver?").body, "Where should we deliver?");
    /// ```
    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Appends a pre-filled field.
    ///
    /// ```
    /// use meta_whatsapp::{AddressField, AddressFieldType, AddressMessage};
    /// let field = AddressField::new(AddressFieldType::Name, "name", "Ada");
    /// assert_eq!(AddressMessage::new("DE").value(field).values.len(), 1);
    /// ```
    #[must_use]
    pub fn value(mut self, value: AddressField) -> Self {
        self.values.push(value);
        self
    }

    /// Adds a pre-filled field from its parts.
    ///
    /// ```
    /// use meta_whatsapp::{AddressFieldType, AddressMessage};
    /// assert_eq!(
    ///     AddressMessage::new("DE")
    ///         .field(AddressFieldType::Address, "city", "Berlin")
    ///         .values
    ///         .len(),
    ///     1,
    /// );
    /// ```
    #[must_use]
    pub fn field(
        self,
        field_type: AddressFieldType,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.value(AddressField::new(field_type, name, value))
    }

    /// Appends a saved address.
    ///
    /// ```
    /// use meta_whatsapp::{AddressMessage, SavedAddress};
    /// assert_eq!(AddressMessage::new("DE").saved_address(SavedAddress::new("home")).saved_addresses.len(), 1);
    /// ```
    #[must_use]
    pub fn saved_address(mut self, address: SavedAddress) -> Self {
        self.saved_addresses.push(address);
        self
    }

    /// Checks the country, body, pre-filled values and saved addresses.
    ///
    /// ```
    /// use meta_whatsapp::AddressMessage;
    ///
    /// assert!(AddressMessage::new("DE").validate().is_ok());
    /// assert!(AddressMessage::new("Germany").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a malformed country, oversized body, too many
    /// values/saved addresses, or an invalid field.
    pub fn validate(&self) -> Result<()> {
        validate_body("interactive.body.text", &self.body)?;
        if self.country.len() != 2 || !self.country.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(Error::validation(
                "interactive.action.parameters.country",
                "must be a two-letter uppercase ISO 3166-1 code",
            ));
        }
        if self.values.len() > ADDRESS_VALUES_MAX {
            return Err(Error::validation(
                "interactive.action.parameters.values",
                format!(
                    "must contain at most {ADDRESS_VALUES_MAX} values, got {}",
                    self.values.len()
                ),
            ));
        }
        if self.saved_addresses.len() > ADDRESS_SAVED_MAX {
            return Err(Error::validation(
                "interactive.action.parameters.saved_addresses",
                format!(
                    "must contain at most {ADDRESS_SAVED_MAX} saved addresses, got {}",
                    self.saved_addresses.len()
                ),
            ));
        }
        for value in &self.values {
            value.validate()?;
        }
        for address in &self.saved_addresses {
            address.validate()?;
        }
        Ok(())
    }

    pub(crate) fn render(&self) -> Value {
        let mut parameters = Map::new();
        parameters.insert("country".into(), Value::String(self.country.clone()));
        if !self.values.is_empty() {
            parameters.insert(
                "values".into(),
                Value::Array(self.values.iter().map(AddressField::render).collect()),
            );
        }
        if !self.saved_addresses.is_empty() {
            parameters.insert(
                "saved_addresses".into(),
                Value::Array(
                    self.saved_addresses
                        .iter()
                        .map(SavedAddress::render)
                        .collect(),
                ),
            );
        }

        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("address_message".into()));
        interactive.insert("body".into(), json!({ "text": self.body }));
        interactive.insert(
            "action".into(),
            json!({ "name": "address_message", "parameters": Value::Object(parameters) }),
        );
        Value::Object(interactive)
    }
}

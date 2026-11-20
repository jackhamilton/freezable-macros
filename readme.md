Support macros for the freezable-trait crate.

`#[freezable]` adds serde support and a `Default` implementation to a named
struct. Field defaults use struct field initializer syntax:

```rust
#[freezable]
pub struct Theme {
    pub term: String = "plugin-name",
    pub ghostty_theme: Option<String>,
    pub nvim_plugin: Option<String>,
    pub nvim_themename: String = "nvim-plugin-colorscheme",
}
```

Fields without an explicit value use `Default::default()`. An explicit value
is converted with `.into()`, so string literals work for `String` fields.
Every field type without `Default` must have an explicit value. The macro
generates `Default` for the struct, so remove any handwritten `Default`
implementation for it. Fields marked with `#[cfg]` or a `#[cfg_attr]` that
enables `cfg` must also have an explicit value; this keeps the generated bounds
valid on stable Rust.

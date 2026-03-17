use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    source: Cow<'static, str>,
}

impl PromptTemplate {
    pub fn new(source: impl Into<Cow<'static, str>>) -> Self {
        Self {
            source: source.into(),
        }
    }

    pub fn render<F>(&self, mut resolver: F) -> Result<String, TemplateRenderError>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let mut rendered = String::new();
        let mut remaining = self.source.as_ref();

        while let Some(start) = remaining.find("{{") {
            let (prefix, after_start) = remaining.split_at(start);
            rendered.push_str(prefix);

            let after_start = &after_start[2..];
            let Some(end) = after_start.find("}}") else {
                return Err(TemplateRenderError::UnclosedPlaceholder);
            };

            let (placeholder, after_end) = after_start.split_at(end);
            let key = placeholder.trim();
            if key.is_empty() {
                return Err(TemplateRenderError::EmptyPlaceholder);
            }

            let value = resolver(key)
                .ok_or_else(|| TemplateRenderError::MissingVariable(key.to_string()))?;
            rendered.push_str(&value);
            remaining = &after_end[2..];
        }

        rendered.push_str(remaining);
        Ok(rendered)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateRenderError {
    EmptyPlaceholder,
    MissingVariable(String),
    UnclosedPlaceholder,
}

impl std::fmt::Display for TemplateRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateRenderError::EmptyPlaceholder => {
                write!(f, "template contains an empty placeholder")
            }
            TemplateRenderError::MissingVariable(variable) => {
                write!(f, "template variable '{variable}' is missing")
            }
            TemplateRenderError::UnclosedPlaceholder => {
                write!(f, "template contains an unclosed placeholder")
            }
        }
    }
}

impl std::error::Error for TemplateRenderError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_variables_in_template() {
        let template = PromptTemplate::new("hello {{ name }}");
        let rendered = template
            .render(|key| (key == "name").then(|| "world".to_string()))
            .expect("template should render");

        assert_eq!(rendered, "hello world");
    }

    #[test]
    fn errors_when_variable_is_missing() {
        let template = PromptTemplate::new("hello {{ name }}");
        let err = template
            .render(|_| None)
            .expect_err("missing variable should fail");

        assert_eq!(
            err,
            TemplateRenderError::MissingVariable("name".to_string())
        );
    }
}

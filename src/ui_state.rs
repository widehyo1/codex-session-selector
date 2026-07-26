#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecVisibility {
    Hidden,
    Shown,
}

impl ExecVisibility {
    pub(crate) fn from_include_exec(include_exec: bool) -> Self {
        if include_exec {
            Self::Shown
        } else {
            Self::Hidden
        }
    }

    pub(crate) fn is_shown(self) -> bool {
        self == Self::Shown
    }

    pub(crate) fn toggle(&mut self) {
        *self = match *self {
            Self::Hidden => Self::Shown,
            Self::Shown => Self::Hidden,
        };
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Shown => "shown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_maps_cli_bool_and_toggles() {
        let mut hidden = ExecVisibility::from_include_exec(false);
        assert_eq!(hidden, ExecVisibility::Hidden);
        assert!(!hidden.is_shown());
        assert_eq!(hidden.label(), "hidden");

        hidden.toggle();
        assert_eq!(hidden, ExecVisibility::Shown);
        assert!(hidden.is_shown());
        assert_eq!(hidden.label(), "shown");

        hidden.toggle();
        assert_eq!(hidden, ExecVisibility::Hidden);
    }
}

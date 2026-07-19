pub trait ApplyConditional: Sized {
    #[must_use]
    fn apply_if_some<T>(self, value: Option<T>, f: impl FnOnce(Self, T) -> Self) -> Self {
        match value {
            Some(value) => f(self, value),
            None => self,
        }
    }

    #[must_use]
    fn apply_if_ok<T, E>(self, result: Result<T, E>, f: impl FnOnce(Self, T) -> Self) -> Self {
        match result {
            Ok(value) => f(self, value),
            Err(_) => self,
        }
    }

    #[must_use]
    fn apply_if_ok_ref<'a, T, E>(
        self,
        result: &'a Result<T, E>,
        f: impl FnOnce(Self, &'a T) -> Self,
    ) -> Self {
        match result {
            Ok(value) => f(self, value),
            Err(_) => self,
        }
    }

    #[must_use]
    fn apply_if_some_ref<'a, T>(
        self,
        option: &'a Option<T>,
        f: impl FnOnce(Self, &'a T) -> Self,
    ) -> Self {
        match option {
            Some(value) => f(self, value),
            None => self,
        }
    }
}

impl<T> ApplyConditional for T {}

use astrcode_client::{
    AstrcodeClientError, AstrcodeClientErrorKind, AstrcodeConversationBannerErrorCodeDto,
    AstrcodeConversationErrorEnvelopeDto,
};

use super::AppController;

impl<T> AppController<T> {
    pub(super) fn apply_status_error(&mut self, error: AstrcodeClientError) {
        self.state.set_error_status(error.message);
    }

    pub(super) fn apply_hydration_error(&mut self, error: AstrcodeClientError) {
        match error.kind {
            AstrcodeClientErrorKind::AuthExpired
            | AstrcodeClientErrorKind::CursorExpired
            | AstrcodeClientErrorKind::StreamDisconnected
            | AstrcodeClientErrorKind::TransportUnavailable
            | AstrcodeClientErrorKind::UnexpectedResponse => self.apply_banner_error(error),
            _ => self.apply_status_error(error),
        }
    }

    pub(super) fn apply_banner_error(&mut self, error: AstrcodeClientError) {
        self.state
            .set_banner_error(AstrcodeConversationErrorEnvelopeDto {
                code: match error.kind {
                    AstrcodeClientErrorKind::AuthExpired => {
                        AstrcodeConversationBannerErrorCodeDto::AuthExpired
                    },
                    AstrcodeClientErrorKind::CursorExpired => {
                        AstrcodeConversationBannerErrorCodeDto::CursorExpired
                    },
                    AstrcodeClientErrorKind::StreamDisconnected
                    | AstrcodeClientErrorKind::TransportUnavailable
                    | AstrcodeClientErrorKind::PermissionDenied
                    | AstrcodeClientErrorKind::Validation
                    | AstrcodeClientErrorKind::NotFound
                    | AstrcodeClientErrorKind::Conflict
                    | AstrcodeClientErrorKind::UnexpectedResponse => {
                        AstrcodeConversationBannerErrorCodeDto::StreamDisconnected
                    },
                },
                message: error.message.clone(),
                rehydrate_required: matches!(error.kind, AstrcodeClientErrorKind::CursorExpired),
                details: error.details,
            });
        self.state.set_error_status(error.message);
    }
}

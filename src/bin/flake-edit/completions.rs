use crate::app::FlakeEdit;
use crate::error::FeError;
use crate::CliArgs;


/// Initialize a version of the editor, that only ever logs errors and will
/// not exit on an error
pub fn init_completion_editor(args: CliArgs) ->  Result<FlakeEdit, FeError> {
    let app = FlakeEdit::init(&args)?;
    Ok(app)
}


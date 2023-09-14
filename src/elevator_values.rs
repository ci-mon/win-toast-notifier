pub mod elevator_values {
    use lazy_static::lazy_static;
    use named_pipe::PipeClient;
    use std::sync::{Arc, LockResult, Mutex};
    lazy_static! {
        pub static ref OUT_PIPE: Arc<Mutex<Option<PipeClient>>> = <_>::default();
    }
}

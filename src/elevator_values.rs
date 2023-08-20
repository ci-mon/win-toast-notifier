pub mod elevator_values {
    use lazy_static::lazy_static;
    use std::sync::{Arc, LockResult, Mutex};
    use named_pipe::PipeClient;
    lazy_static! {
        pub static ref OUT_PIPE: Arc<Mutex<Option<PipeClient>>> = <_>::default();
    }
}
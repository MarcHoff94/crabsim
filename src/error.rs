#[derive(Debug)]
pub struct InitError;

#[derive(Debug)]
pub struct RunTimeError;

#[derive(Debug)]
pub enum MaterialflowError {
    EntranceDenied,
    NoMU,
}

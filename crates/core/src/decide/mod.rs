mod ingress;
mod tick;
mod review;
mod global;
mod driver;
mod scheduler;
mod sender;

pub mod builder;

use crate::command::Command;
use crate::config::CoreConfig;
use crate::event::Event;
use crate::state::StateView;

pub fn decide(state: &StateView, command: &Command, config: &CoreConfig) -> Vec<Event> {
    match command {
        Command::Ingress(cmd) => ingress::decide_ingress(state, cmd, config),
        Command::Tick(cmd) => tick::decide_tick(state, cmd, config),
        Command::ReviewAction(cmd) => review::decide_review_action(state, cmd, config),
        Command::GlobalAction(cmd) => global::decide_global_action(state, cmd, config),
        Command::DriverEvent(event) => driver::decide_driver_event(state, event, config),
    }
}

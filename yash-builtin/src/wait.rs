// This file is part of yash, an extended POSIX shell.
// Copyright (C) 2022 WATANABE Yuki
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Wait built-in
//!
//! The **`wait`** built-in waits for asynchronous jobs to finish.
//!
//! # Syntax
//!
//! ```sh
//! wait [job_id_or_process_id...]
//! ```
//!
//! # Semantics
//!
//! If you specify one or more operands, the built-in waits for the specified
//! job to finish. Otherwise, the built-in waits for all existing asynchronous
//! jobs.
//!
//! If the job is already finished, the built-in returns without waiting. If the
//! job is job-controlled (that is, running in its own process group), it is
//! considered finished not only when it has exited but also when it has been
//! suspended.
//!
//! # Options
//!
//! None
//!
//! # Operands
//!
//! An operand can be a job ID or decimal process ID, specifying which job to
//! wait for.
//!
//! ## Job ID
//!
//! TODO Elaborate on syntax of job ID
//!
//! ## Process ID
//!
//! A process ID is a non-negative decimal integer.
//!
//! If a process ID does not specify a job contained in the [`JobSet`] of the
//! current environment, the built-in treats it as an existing job that has
//! already finished with exit status 127.
//!
//! # Exit status
//!
//! If you specify one or more operands, the built-in returns the exit status of
//! the job specified by the last operand. If there is no operand, the exit
//! status is 0 regardless of the awaited jobs.
//!
//! If the built-in was interrupted by a signal, the exit status indicates the
//! signal.
//!
//! # Errors
//!
//! TBD
//!
//! # Portability
//!
//! The wait built-in is contained in the POSIX standard.
//!
//! The exact value of an exit status resulting from a signal is
//! implementation-dependent.

use crate::common::print_error_message;
use crate::common::syntax::parse_arguments;
use crate::common::syntax::Mode;
use std::num::ParseIntError;
use thiserror::Error;
use yash_env::builtin::Result;
use yash_env::job::JobSet;
use yash_env::job::Pid;
use yash_env::job::WaitStatus;
use yash_env::semantics::ExitStatus;
use yash_env::semantics::Field;
use yash_env::system::Errno;
use yash_env::Env;
use yash_syntax::source::pretty::Annotation;
use yash_syntax::source::pretty::AnnotationType;
use yash_syntax::source::pretty::MessageBase;

// TODO Split into syntax and semantics submodules

// TODO Parse as a job ID if an operand starts with %
// TODO Treat an unknown job as terminated with exit status 127
// TODO Treat a suspended job as terminated if it is job-controlled.
// TODO Interruption by trap
// TODO Allow interrupting with SIGINT if interactive

#[derive(Clone, Debug, Eq, Error, PartialEq)]
enum JobSpecError {
    #[error("{}: {}", .0.value, .1)]
    ParseInt(Field, ParseIntError),
    #[error("{}: non-positive process ID", .0.value)]
    NonPositive(Field),
}

impl JobSpecError {
    fn field(&self) -> &Field {
        match self {
            JobSpecError::ParseInt(field, _) => field,
            JobSpecError::NonPositive(field) => field,
        }
    }
}

impl MessageBase for JobSpecError {
    fn message_title(&self) -> std::borrow::Cow<str> {
        "invalid job specification".into()
    }
    fn main_annotation(&self) -> Annotation {
        Annotation::new(
            AnnotationType::Error,
            self.to_string().into(),
            &self.field().origin,
        )
    }
}

fn to_job_result(status: WaitStatus) -> Option<(Pid, ExitStatus)> {
    match status {
        WaitStatus::Exited(pid, exit_status_value) => Some((pid, ExitStatus(exit_status_value))),
        WaitStatus::Signaled(_pid, _signal, _core_dumped) => todo!("handle signaled job"),
        WaitStatus::Stopped(_pid, _signal) => todo!("handle stopped job"),
        WaitStatus::Continued(_pid) => todo!("handle continued job"),
        _ => None,
    }
}

fn remove_finished_jobs(jobs: &mut JobSet) {
    jobs.drain_filter(|_index, job| to_job_result(job.status).is_some());
}

async fn wait_for_all_jobs(env: &mut Env) -> ExitStatus {
    loop {
        remove_finished_jobs(&mut env.jobs);
        if env.jobs.is_empty() {
            break;
        }
        match env.wait_for_subshell(Pid::from_raw(-1)).await {
            // When the shell creates a subshell, it inherits jobs of the
            // parent shell, but those jobs are not child processes of the
            // subshell. The wait built-in invoked in the subshell needs to
            // ignore such jobs.
            Err(Errno::ECHILD) => break,

            Err(Errno::EINTR) => todo!("signal interruption"),
            Err(_) => todo!("handle unexpected error"),
            Ok(_) => (),
        }
    }
    ExitStatus::SUCCESS
}

async fn wait_for_job(env: &mut Env, index: usize) -> ExitStatus {
    let exit_status = loop {
        let job = env.jobs.get(index).unwrap();
        if let Some((_pid, exit_status)) = to_job_result(job.status) {
            break exit_status;
        }
        match env.wait_for_subshell(Pid::from_raw(-1)).await {
            // When the shell creates a subshell, it inherits jobs of the parent
            // shell, but those jobs are not child processes of the subshell.
            // The wait built-in invoked in the subshell needs to ignore such
            // jobs.
            Err(Errno::ECHILD) => break ExitStatus::NOT_FOUND,
            Err(Errno::EINTR) => todo!("signal interruption"),
            Err(_) => todo!("handle unexpected error"),
            Ok(_) => (),
        }
    };
    env.jobs.remove(index);
    exit_status
}

async fn wait_for_each_job(env: &mut Env, job_specs: Vec<Field>) -> Result {
    let mut exit_status = ExitStatus::SUCCESS;

    for job_spec in job_specs {
        let pid = match job_spec.value.parse() {
            Ok(pid) if pid > 0 => Pid::from_raw(pid),
            Ok(_) => return print_error_message(env, &JobSpecError::NonPositive(job_spec)).await,
            Err(e) => return print_error_message(env, &JobSpecError::ParseInt(job_spec, e)).await,
        };

        exit_status = if let Some(index) = env.jobs.find_by_pid(pid) {
            wait_for_job(env, index).await
        } else {
            ExitStatus::NOT_FOUND
        };
    }

    exit_status.into()
}

/// Entry point for executing the `wait` built-in
pub async fn main(env: &mut Env, args: Vec<Field>) -> Result {
    let (_options, operands) = match parse_arguments(&[], Mode::with_env(env), args) {
        Ok(result) => result,
        Err(error) => return print_error_message(env, &error).await,
    };

    if operands.is_empty() {
        wait_for_all_jobs(env).await.into()
    } else {
        wait_for_each_job(env, operands).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::assert_stderr;
    use crate::tests::in_virtual_system;
    use assert_matches::assert_matches;
    use futures_util::FutureExt;
    use std::ops::ControlFlow::Continue;
    use std::rc::Rc;
    use yash_env::job::Job;
    use yash_env::stack::Frame;
    use yash_env::subshell::Subshell;
    use yash_env::system::r#virtual::ProcessState;
    use yash_env::VirtualSystem;

    // A child process that is not managed as a job in the shell's JobSet may
    // happen if the process running the shell performed a fork before "exec"ing
    // into the shell. Such a process is a child of the shell but is not known
    // by the shell.

    #[test]
    fn wait_no_operands_no_jobs() {
        in_virtual_system(|mut env, _state| async move {
            // Start a child process, but don't turn it into a job.
            let subshell = Subshell::new(|_, _| Box::pin(futures_util::future::pending()));
            subshell.start(&mut env).await.unwrap();

            let result = main(&mut env, vec![]).await;
            assert_eq!(result, Result::new(ExitStatus::SUCCESS));
        })
    }

    #[test]
    fn wait_no_operands_some_running_jobs() {
        in_virtual_system(|mut env, state| async move {
            for i in 1..=2 {
                let subshell = Subshell::new(move |env, _job_control| {
                    Box::pin(async move {
                        env.exit_status = ExitStatus(i);
                        Continue(())
                    })
                });
                let pid = subshell.start(&mut env).await.unwrap().0;
                env.jobs.add(Job::new(pid));
            }

            let result = main(&mut env, vec![]).await;
            assert_eq!(result, Result::new(ExitStatus::SUCCESS));
            assert_eq!(env.jobs.len(), 0);

            let state = state.borrow();
            for (cpid, process) in &state.processes {
                if *cpid != env.main_pid {
                    assert!(!process.state_has_changed());
                    assert_matches!(process.state(), ProcessState::Exited(exit_status) => {
                        assert_ne!(exit_status, ExitStatus::SUCCESS);
                    });
                }
            }
        })
    }

    #[test]
    fn wait_no_operands_some_finished_jobs() {
        let mut env = Env::new_virtual();

        // Add a job that has already exited.
        let pid = Pid::from_raw(10);
        let mut job = Job::new(pid);
        job.status = WaitStatus::Exited(pid, 42);
        let index = env.jobs.add(job);

        let result = main(&mut env, vec![]).now_or_never().unwrap();
        assert_eq!(result, Result::new(ExitStatus::SUCCESS));
        assert_eq!(env.jobs.get(index), None);
    }

    #[test]
    fn wait_no_operands_false_job() {
        let mut env = Env::new_virtual();

        // Add a running job that is not a proper subshell.
        let index = env.jobs.add(Job::new(Pid::from_raw(1)));

        let result = main(&mut env, vec![]).now_or_never().unwrap();
        assert_eq!(result, Result::new(ExitStatus::SUCCESS));
        assert_eq!(env.jobs.get(index).unwrap().status, WaitStatus::StillAlive);
    }

    #[test]
    fn wait_some_operands_no_jobs() {
        in_virtual_system(|mut env, _state| async move {
            // Start a child process, but don't turn it into a job.
            let subshell = Subshell::new(|_, _| Box::pin(futures_util::future::pending()));
            let pid = subshell.start(&mut env).await.unwrap().0;

            let args = Field::dummies([pid.to_string()]);
            let result = main(&mut env, args).await;
            assert_eq!(result, Result::new(ExitStatus::NOT_FOUND));
        })
    }

    #[test]
    fn wait_some_operands_some_running_jobs() {
        in_virtual_system(|mut env, state| async move {
            let mut pids = Vec::new();
            for i in 5..=6 {
                let subshell = Subshell::new(move |env, _job_control| {
                    Box::pin(async move {
                        env.exit_status = ExitStatus(i);
                        Continue(())
                    })
                });
                let pid = subshell.start(&mut env).await.unwrap().0;
                pids.push(pid.to_string());
                env.jobs.add(Job::new(pid));
            }

            let args = Field::dummies(pids);
            let result = main(&mut env, args).await;
            assert_eq!(result, Result::new(ExitStatus(6)));
            assert_eq!(env.jobs.len(), 0);

            let state = state.borrow();
            for (cpid, process) in &state.processes {
                if *cpid != env.main_pid {
                    assert!(!process.state_has_changed());
                    assert_matches!(process.state(), ProcessState::Exited(exit_status) => {
                        assert_ne!(exit_status, ExitStatus::SUCCESS);
                    });
                }
            }
        })
    }

    #[test]
    fn wait_some_operands_some_finished_job() {
        let mut env = Env::new_virtual();

        // Add a job that has already exited.
        let pid = Pid::from_raw(7);
        let mut job = Job::new(pid);
        job.status = WaitStatus::Exited(pid, 17);
        let index = env.jobs.add(job);

        let args = Field::dummies([pid.to_string()]);
        let result = main(&mut env, args).now_or_never().unwrap();
        assert_eq!(result, Result::new(ExitStatus(17)));
        assert_eq!(env.jobs.get(index), None);
    }

    #[test]
    fn wait_some_operands_false_job() {
        let mut env = Env::new_virtual();

        // Add a running job that is not a proper subshell.
        let index = env.jobs.add(Job::new(Pid::from_raw(19)));

        let args = Field::dummies(["19".to_string()]);
        let result = main(&mut env, args).now_or_never().unwrap();
        assert_eq!(result, Result::new(ExitStatus::NOT_FOUND));
        assert_eq!(env.jobs.get(index), None);
    }

    #[test]
    fn wait_unknown_process_id() {
        let mut env = Env::new_virtual();
        let args = Field::dummies(["9999999"]);
        let result = main(&mut env, args).now_or_never().unwrap();
        assert_eq!(result, Result::new(ExitStatus::NOT_FOUND));
    }

    #[test]
    fn non_numeric_operand() {
        let system = VirtualSystem::new();
        let state = Rc::clone(&system.state);
        let mut env = Env::with_system(Box::new(system));
        let mut env = env.push_frame(Frame::Builtin {
            name: Field::dummy("wait"),
            is_special: false,
        });
        let args = Field::dummies(["abc"]);

        let result = main(&mut env, args).now_or_never().unwrap();
        assert_eq!(result, Result::new(ExitStatus::ERROR));
        assert_stderr(&state, |stderr| assert_ne!(stderr, ""));
    }

    #[test]
    fn non_positive_process_id() {
        let system = VirtualSystem::new();
        let state = Rc::clone(&system.state);
        let mut env = Env::with_system(Box::new(system));
        let mut env = env.push_frame(Frame::Builtin {
            name: Field::dummy("wait"),
            is_special: false,
        });
        let args = Field::dummies(["0"]);

        let result = main(&mut env, args).now_or_never().unwrap();
        assert_eq!(result, Result::new(ExitStatus::ERROR));
        assert_stderr(&state, |stderr| assert_ne!(stderr, ""));
    }
}

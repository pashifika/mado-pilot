//! Owned descendant lifetime, bounded file protocol, and private pixel oracle.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use mado_pilot::{FrameDescriptor, OperationContext, PixelExtent, PixelFormat};

use super::completion_cooldown::checkpoint;
use super::contract::{Arguments, CONTROL_TIMEOUT, Check, Failure, api, bounded, require};

const POLL: Duration = Duration::from_millis(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Ack {
    pub(super) sequence: u32,
    pub(super) pid: u32,
    pub(super) window: u64,
    pub(super) extent: PixelExtent,
    pub(super) counter: u32,
    pub(super) state: u32,
}

impl Ack {
    fn parse(row: &str, nonce: &str) -> Check<Self> {
        require(row.len() <= 512 && row.is_ascii(), "fixture-ack-format")?;
        let row = row
            .strip_suffix('\n')
            .ok_or(Failure::Rule("fixture-ack-format"))?;
        let fields: Vec<_> = row.split(' ').collect();
        require(
            fields.len() == 9 && fields[0] == "ACK" && fields[1] == nonce,
            "fixture-ack-identity",
        )?;
        let number = |index: usize| -> Check<u64> {
            require(
                !fields[index].is_empty()
                    && fields[index].bytes().all(|byte| byte.is_ascii_digit()),
                "fixture-ack-format",
            )?;
            fields[index]
                .parse()
                .map_err(|_| Failure::Rule("fixture-ack-format"))
        };
        let small =
            |index| u32::try_from(number(index)?).map_err(|_| Failure::Rule("fixture-ack-format"));
        let ack = Self {
            sequence: small(2)?,
            pid: small(3)?,
            window: number(4)?,
            extent: PixelExtent::new(small(5)?, small(6)?),
            counter: small(7)?,
            state: small(8)?,
        };
        require(
            ack.sequence <= 128
                && ack.pid > 0
                && ack.window > 0
                && ack.counter > 0
                && ack.state <= 1,
            "fixture-ack-values",
        )?;
        require(
            [PixelExtent::new(960, 576), PixelExtent::new(1040, 640)].contains(&ack.extent),
            "fixture-ack-geometry",
        )?;
        Ok(ack)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Marker {
    pub(super) counter: u32,
    pub(super) state: u32,
}

pub(super) fn marker(descriptor: FrameDescriptor, bytes: &[u8], nonce: u64) -> Check<Marker> {
    require(
        descriptor.format() == PixelFormat::Bgra8
            && descriptor.extent().width() >= 512
            && descriptor.extent().height() >= 560
            && bytes.len() == descriptor.byte_len(),
        "marker-layout",
    )?;
    let mut words = [0_u64; 2];
    for bit in 0..128 {
        let offset = 552_usize
            .checked_mul(descriptor.stride())
            .and_then(|row| row.checked_add((bit * 4 + 2) * 4))
            .ok_or(Failure::Rule("marker-layout"))?;
        let pixel = bytes
            .get(offset..offset + 4)
            .ok_or(Failure::Rule("marker-layout"))?;
        // Color management can lift the low channel without changing blue/yellow dominance.
        let [blue, green, red] = [pixel[0], pixel[1], pixel[2]].map(u16::from);
        let one = if red >= blue + 96 && green >= blue + 96 {
            1
        } else if blue >= red + 96 && blue >= green + 96 {
            0
        } else {
            return Err(Failure::Rule("owned-pixel-color"));
        };
        words[bit / 64] = (words[bit / 64] << 1) | one;
    }
    require(words[0] == nonce, "owned-pixel-nonce")?;
    let marker = Marker {
        counter: u32::try_from(words[1] >> 32).map_err(|_| Failure::Rule("owned-pixel-state"))?,
        state: u32::try_from(words[1] & u64::from(u32::MAX))
            .map_err(|_| Failure::Rule("owned-pixel-state"))?,
    };
    require(marker.counter > 0 && marker.state <= 1, "owned-pixel-state")?;
    Ok(marker)
}

fn read_small(path: &Path) -> Check<Option<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Failure::Rule("control-read")),
    };
    let mut bytes = Vec::with_capacity(513);
    file.take(513)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Rule("control-read"))?;
    require(bytes.len() <= 512, "control-size")?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| Failure::Rule("control-encoding"))
}

fn atomic_write(root: &Path, name: &str, bytes: &[u8]) -> Check<()> {
    let temporary = root.join(format!("{name}.consumer-tmp"));
    fs::write(&temporary, bytes).map_err(|_| Failure::Rule("control-write"))?;
    fs::rename(temporary, root.join(name)).map_err(|_| Failure::Rule("control-publish"))
}

pub(super) struct Fixture {
    child: Option<Child>,
    root: PathBuf,
    nonce: String,
    pub(super) token: u64,
    pub(super) ack: Ack,
}

impl Fixture {
    pub(super) fn spawn(arguments: &Arguments, operation: &OperationContext) -> Check<Self> {
        api(checkpoint(operation))?;
        let root = arguments
            .root
            .canonicalize()
            .map_err(|_| Failure::NotRun("control-root-unavailable"))?;
        require(
            root.is_dir() && root.components().any(|part| part.as_os_str() == ".rasen"),
            "control-root-scope",
        )?;
        for name in [
            "ack",
            "command",
            "stop",
            "fixture-error",
            "fixture-metrics.json",
        ] {
            require(!root.join(name).exists(), "control-root-not-fresh")?;
        }
        let fixture = arguments
            .fixture
            .canonicalize()
            .map_err(|_| Failure::NotRun("fixture-unavailable"))?;
        let image = arguments
            .image
            .canonicalize()
            .map_err(|_| Failure::NotRun("image-unavailable"))?;
        require(fixture.is_file() && image.is_file(), "fixture-prerequisite")?;
        let child = Command::new(fixture)
            .arg(&root)
            .arg(&arguments.nonce)
            .arg(image)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Failure::NotRun("fixture-launch"))?;
        let owned = Self {
            ack: Ack {
                sequence: 0,
                pid: child.id(),
                window: 0,
                extent: PixelExtent::new(960, 576),
                counter: 0,
                state: 0,
            },
            child: Some(child),
            root,
            nonce: arguments.nonce.clone(),
            token: arguments.token,
        };
        Ok(owned)
    }

    pub(super) fn ready(&mut self, operation: &OperationContext) -> Check<()> {
        self.ack = self.wait_ack(0, &bounded(operation, CONTROL_TIMEOUT)?)?;
        require(
            self.ack.state == 0 && self.ack.extent == PixelExtent::new(960, 576),
            "fixture-startup-state",
        )
    }

    pub(super) fn title(&self) -> String {
        format!("MadoPilot Pacing {}", self.nonce)
    }

    pub(super) fn healthy(&mut self) -> Check<()> {
        require(!self.root.join("fixture-error").exists(), "fixture-failed")?;
        let child = self.child.as_mut().ok_or(Failure::Rule("fixture-exited"))?;
        require(
            child
                .try_wait()
                .map_err(|_| Failure::Rule("fixture-wait"))?
                .is_none(),
            "fixture-exited",
        )
    }

    fn wait_ack(&mut self, sequence: u32, operation: &OperationContext) -> Check<Ack> {
        loop {
            api(checkpoint(operation))?;
            require(!self.root.join("fixture-error").exists(), "fixture-failed")?;
            if let Some(row) = read_small(&self.root.join("ack"))? {
                let ack = Ack::parse(&row, &self.nonce)?;
                require(
                    ack.pid == self.ack.pid
                        && (self.ack.window == 0 || ack.window == self.ack.window),
                    "fixture-child-identity",
                )?;
                require(
                    ack.sequence <= sequence && ack.sequence >= self.ack.sequence,
                    "fixture-ack-sequence",
                )?;
                if ack.sequence == sequence {
                    return Ok(ack);
                }
            }
            self.healthy()?;
            std::thread::sleep(POLL);
        }
    }

    pub(super) fn command(
        &mut self,
        command: &'static str,
        operation: &OperationContext,
    ) -> Check<Ack> {
        api(checkpoint(operation))?;
        self.healthy()?;
        require(
            matches!(
                command,
                "blank"
                    | "show"
                    | "pulse"
                    | "animate"
                    | "pause"
                    | "burst"
                    | "resize"
                    | "reset-stats"
                    | "stats"
                    | "close-window"
                    | "quit"
            ),
            "fixture-command",
        )?;
        let sequence = self
            .ack
            .sequence
            .checked_add(1)
            .filter(|value| *value <= 128)
            .ok_or(Failure::Rule("fixture-command-limit"))?;
        let row = format!("{} {sequence} {command}\n", self.nonce);
        require(row.len() <= 128, "fixture-command-limit")?;
        atomic_write(&self.root, "command", row.as_bytes())?;
        let ack = self.wait_ack(sequence, &bounded(operation, CONTROL_TIMEOUT)?)?;
        require(ack.counter >= self.ack.counter, "fixture-counter-regressed")?;
        self.ack = ack;
        Ok(ack)
    }

    pub(super) fn shutdown(&mut self) -> Check<()> {
        if self.child.is_none() {
            return Ok(());
        }
        let cleanup = api(OperationContext::new().with_timeout(Duration::from_secs(5)))?;
        let mut clean = !self.root.join("fixture-error").exists();
        let running = self
            .child
            .as_mut()
            .ok_or(Failure::Rule("fixture-cleanup"))?
            .try_wait()
            .map_err(|_| Failure::Rule("fixture-cleanup"))?;
        if running.is_some() {
            self.child.take();
            return Err(Failure::Rule("fixture-exited-before-cleanup"));
        }
        if self.command("quit", &cleanup).is_err() {
            clean = false;
            let _ = atomic_write(&self.root, "stop", b"stop\n");
        }
        while cleanup.interruption().is_none() {
            if let Some(status) = self
                .child
                .as_mut()
                .ok_or(Failure::Rule("fixture-cleanup"))?
                .try_wait()
                .map_err(|_| Failure::Rule("fixture-cleanup"))?
            {
                self.child.take();
                return require(
                    clean && status.success() && !self.root.join("fixture-error").exists(),
                    "fixture-cleanup",
                );
            }
            std::thread::sleep(POLL);
        }
        let _ = atomic_write(&self.root, "stop", b"stop\n");
        self.child
            .as_mut()
            .ok_or(Failure::Rule("fixture-cleanup"))?
            .kill()
            .map_err(|_| Failure::Rule("fixture-kill"))?;
        let reap = api(OperationContext::new().with_timeout(Duration::from_secs(1)))?;
        while reap.interruption().is_none() {
            if self
                .child
                .as_mut()
                .ok_or(Failure::Rule("fixture-cleanup"))?
                .try_wait()
                .map_err(|_| Failure::Rule("fixture-cleanup"))?
                .is_some()
            {
                self.child.take();
                break;
            }
            std::thread::sleep(POLL);
        }
        Err(Failure::Rule("fixture-cleanup"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_refuses_foreign_owner_tokens_and_malformed_records() {
        let nonce = "0123456789abcdef";
        let valid = "ACK 0123456789abcdef 0 42 72 960 576 1 0\n";
        let ack = Ack::parse(valid, nonce).expect("startup ack");
        assert_eq!((ack.pid, ack.window, ack.counter), (42, 72, 1));
        assert!(Ack::parse(valid, "fedcba9876543210").is_err());
        for invalid in [
            "ACK 0123456789abcdef 129 42 72 960 576 1 0\n",
            "ACK 0123456789abcdef 0 42 72 960 576 0 0\n",
            "ACK 0123456789abcdef 0 42 72 960 576 1 2\n",
            "ACK 0123456789abcdef 0 42 72 960 575 1 0\n",
            "ACK 0123456789abcdef 0 42 72 960 576 1 0\nextra",
        ] {
            assert!(Ack::parse(invalid, nonce).is_err());
        }
    }

    #[test]
    fn pixel_oracle_checks_all_nonce_counter_and_state_bits() {
        let descriptor = FrameDescriptor::new(PixelExtent::new(960, 576), PixelFormat::Bgra8, 4096)
            .expect("padded layout");
        let nonce = 0x0123_4567_89ab_cdef;
        let payload = (u64::from(0x1234_5678_u32) << 32) | 1;
        let mut bytes = vec![0; descriptor.byte_len()];
        for bit in 0..128 {
            let word = if bit < 64 { nonce } else { payload };
            let one = (word >> (63 - bit % 64)) & 1 != 0;
            let offset = 552 * descriptor.stride() + (bit * 4 + 2) * 4;
            bytes[offset..offset + 4].copy_from_slice(if one {
                &[0, 255, 255, 255]
            } else {
                &[255, 0, 0, 255]
            });
        }
        assert_eq!(
            marker(descriptor, &bytes, nonce).expect("owned marker"),
            Marker {
                counter: 0x1234_5678,
                state: 1
            }
        );
        assert!(marker(descriptor, &bytes, nonce ^ 1).is_err());
        assert!(marker(descriptor, &bytes[..bytes.len() - 1], nonce).is_err());
        let high_state_bit = 552 * descriptor.stride() + (96 * 4 + 2) * 4;
        bytes[high_state_bit..high_state_bit + 4].copy_from_slice(&[0, 255, 255, 255]);
        assert!(marker(descriptor, &bytes, nonce).is_err());
        bytes[high_state_bit..high_state_bit + 4].copy_from_slice(&[255, 0, 0, 255]);
        let offset = 552 * descriptor.stride() + (127 * 4 + 2) * 4;
        bytes[offset..offset + 4].fill(128);
        assert!(marker(descriptor, &bytes, nonce).is_err());
    }

    #[test]
    fn color_managed_marker_channels_preserve_ownership_but_erasure_does_not() {
        let descriptor = FrameDescriptor::new(PixelExtent::new(960, 576), PixelFormat::Bgra8, 3840)
            .expect("marker layout");
        let nonce = u64::MAX;
        let payload = (2_u64 << 32) | 1;
        let mut bytes = vec![0; descriptor.byte_len()];
        for bit in 0..128 {
            let word = if bit < 64 { nonce } else { payload };
            let one = (word >> (63 - bit % 64)) & 1 != 0;
            let offset = 552 * descriptor.stride() + (bit * 4 + 2) * 4;
            bytes[offset..offset + 4].copy_from_slice(if one {
                &[84, 255, 255, 255]
            } else {
                &[255, 64, 64, 255]
            });
        }
        assert_eq!(
            marker(descriptor, &bytes, nonce).expect("color-managed owned marker"),
            Marker {
                counter: 2,
                state: 1
            }
        );
        let offset = 552 * descriptor.stride() + 2 * 4;
        bytes[offset..offset + 4].fill(255);
        assert!(marker(descriptor, &bytes, nonce).is_err());
    }
}

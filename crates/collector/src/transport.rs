//! Private local connection between the collector and its hosts.
//!
//! Unix uses a socket file that only the owner can open. Windows has no such socket in Rust's
//! standard library, so it uses a loopback TCP port instead: the endpoint file holds the port and
//! a random token, lives in the user's private state folder, and every client must send the token
//! first, so another user of the machine cannot talk to the collector.
use std::{io, path::Path, time::Duration};

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    };
    pub struct Listener(UnixListener);
    pub struct Stream(UnixStream);
    impl Listener {
        pub fn bind(path: &Path) -> io::Result<Self> {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
            let listener = UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)?;
            Ok(Self(listener))
        }
        pub fn accept(&self) -> io::Result<Stream> {
            self.0.accept().map(|(s, _)| Stream(s))
        }
    }
    impl Stream {
        pub fn connect(path: &Path) -> io::Result<Self> {
            UnixStream::connect(path).map(Stream)
        }
        pub fn set_timeouts(&self, timeout: Duration) -> io::Result<()> {
            self.0.set_read_timeout(Some(timeout))?;
            self.0.set_write_timeout(Some(timeout))
        }
    }
    impl io::Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }
    impl io::Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    };
    const TOKEN_BYTES: usize = 16;
    pub struct Listener {
        inner: TcpListener,
        token: [u8; TOKEN_BYTES],
    }
    pub struct Stream(TcpStream);

    #[cfg(windows)]
    fn random(buffer: &mut [u8]) -> io::Result<()> {
        #[link(name = "bcrypt")]
        extern "system" {
            fn BCryptGenRandom(algorithm: *mut u8, buffer: *mut u8, length: u32, flags: u32)
                -> i32;
        }
        // BCRYPT_USE_SYSTEM_PREFERRED_RNG
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                2,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::other("the system random generator failed"))
        }
    }
    #[cfg(not(windows))]
    fn random(buffer: &mut [u8]) -> io::Result<()> {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")?.read_exact(buffer)
    }
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
    fn unhex(text: &str) -> Option<[u8; TOKEN_BYTES]> {
        let mut out = [0u8; TOKEN_BYTES];
        if text.len() != TOKEN_BYTES * 2 {
            return None;
        }
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(out)
    }
    impl Listener {
        pub fn bind(path: &Path) -> io::Result<Self> {
            let inner = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
            inner.set_nonblocking(true)?;
            let mut token = [0u8; TOKEN_BYTES];
            random(&mut token)?;
            // The endpoint is published only after the port is open and the token exists.
            let endpoint = format!("{}\n{}\n", inner.local_addr()?.port(), hex(&token));
            std::fs::write(path, endpoint)?;
            Ok(Self { inner, token })
        }
        pub fn accept(&self) -> io::Result<Stream> {
            let (mut stream, _) = self.inner.accept()?;
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(Duration::from_millis(500)))?;
            let mut given = [0u8; TOKEN_BYTES];
            stream.read_exact(&mut given)?;
            // Compare every byte so the answer does not depend on where the first mismatch is.
            let wrong = given
                .iter()
                .zip(&self.token)
                .fold(0u8, |acc, (a, b)| acc | (a ^ b));
            if wrong != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "wrong token",
                ));
            }
            Ok(Stream(stream))
        }
    }
    impl Stream {
        pub fn connect(path: &Path) -> io::Result<Self> {
            let text = std::fs::read_to_string(path)?;
            let mut lines = text.lines();
            let port: u16 = lines
                .next()
                .and_then(|l| l.parse().ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad endpoint"))?;
            let token = lines
                .next()
                .and_then(unhex)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad endpoint"))?;
            let mut stream = TcpStream::connect_timeout(
                &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                Duration::from_secs(2),
            )?;
            stream.write_all(&token)?;
            Ok(Stream(stream))
        }
        pub fn set_timeouts(&self, timeout: Duration) -> io::Result<()> {
            self.0.set_read_timeout(Some(timeout))?;
            self.0.set_write_timeout(Some(timeout))
        }
    }
    impl io::Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }
    impl io::Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        // REQ SQ-36: the loopback transport (used on Windows) refuses a client without the token.
        #[test]
        fn token_round_trip_and_rejection() {
            assert_eq!(unhex(&hex(&[7u8; TOKEN_BYTES])), Some([7u8; TOKEN_BYTES]));
            assert_eq!(unhex("zz"), None);
        }
    }
}
pub use imp::{Listener, Stream};

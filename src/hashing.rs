use std::io::{self, Read, Write};

/// Borrowing tee that updates an external hasher while the underlying reader is consumed.
/// useful when an API takes the reader by value (e.g. `tar::Builder::append_data`).
pub struct BlakeTee<'a, R: Read> {
    inner: R,
    hasher: &'a mut blake3::Hasher,
}

impl<'a, R: Read> BlakeTee<'a, R> {
    pub fn new(inner: R, hasher: &'a mut blake3::Hasher) -> Self {
        Self { inner, hasher }
    }
}

impl<'a, R: Read> Read for BlakeTee<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }
}

/// Write-side tee mirroring `BlakeTee`. used during extract to hash bytes flowing into the file.
pub struct BlakeTeeWriter<'a, W: Write> {
    inner: W,
    hasher: &'a mut blake3::Hasher,
}

impl<'a, W: Write> BlakeTeeWriter<'a, W> {
    pub fn new(inner: W, hasher: &'a mut blake3::Hasher) -> Self {
        Self { inner, hasher }
    }
}

impl<'a, W: Write> Write for BlakeTeeWriter<'a, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

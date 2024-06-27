use std::io::{Read, Write, Result};
use std::net::{TcpStream as TCP, Shutdown};
use std::any::Any;  
use rustls;

pub trait Stream: Read + Write {
    fn as_any(&self) -> &dyn Any;
    fn shutdown(&mut self) -> Result<()>;
}

pub struct TcpStream {
    stream: TCP 
}

impl TcpStream {
    pub fn new(stream: TCP) -> Self {
        TcpStream { stream }
    }
}

impl Stream for TcpStream {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stream.shutdown(Shutdown::Both)
    }
}

impl Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(buf)
    } 
}

impl Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }
    
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()        
    }
}

pub struct TlsTcpStream {
    stream: rustls::StreamOwned<rustls::ClientConnection, TCP>
}

impl TlsTcpStream{
    pub fn new(stream: rustls::StreamOwned<rustls::ClientConnection, TCP>) -> Self {
        TlsTcpStream { stream }
    }
}

impl Stream for TlsTcpStream {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stream.sock.shutdown(Shutdown::Both)
    }
}

impl Read for TlsTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(buf)
    } 
}

impl Write for TlsTcpStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }
    
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()        
    }
}
use std::collections::HashMap;
use super::commons::END_LINE;
use super::super::core::traits::{Parse, ParseError};

#[allow(dead_code)]
#[derive(Debug)]
pub struct Response {
    version: String,
    status_code: u16,
    status_text: String,
    headers: Option<HashMap<String, String>>,
    body: Option<String>
}

impl Response {
    #[allow(dead_code)]
    pub fn new(version: String, status_code: u16, status_text: String, headers: Option<HashMap<String, String>>, body: Option<String>) -> Self {
        Response { version, status_code, headers, status_text, body }
    }

    pub fn get_status_code(&self) -> u16 {
        return self.status_code;
    }

    pub fn header(&self, key: &str) -> Option<String> {
        if self.headers.is_none() { return None };
        let headers = self.headers.clone().unwrap();
        let key_lower = &key.to_lowercase();
        let value = headers.get(key_lower);
        if value.is_none() { return None };    
        return Some(value.unwrap().clone(   ));
    }

    pub fn body(&self) -> Option<&String> {
        self.body.as_ref()
    }

}

// TODO: Parse the rest of the response
impl Parse for Response {
    fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        // Convert into utf-8 String and replace the invalid characters
        let bytes = String::from_utf8_lossy(bytes).replace("�", "");

        let end_header = bytes.find(format!("{}{}", END_LINE, END_LINE).as_str());
        if end_header.is_none() { return Err(ParseError) }

        let end_header = end_header.unwrap();
        let header = &bytes[0..end_header];
        let body = &bytes[end_header + END_LINE.len() * 2..bytes.len()];

        let header_lines:Vec<&str> = header.split(END_LINE).collect();

        // Parse version, status code and status text
        let response_info: Vec<&str> = header_lines[0].split(" ").collect();
        let version = response_info[0].trim().to_string();
        let status_code = response_info[1].trim().parse::<u16>();

        if status_code.is_err() { return Err(ParseError) }

        let status_code = status_code.unwrap();
        let status_text = response_info[2].trim().to_string();
        
        // Parse headers
        let mut headers = HashMap::new();
        let header_lines = &header_lines[1..header_lines.len()];
        for line in header_lines {
            let index = line.find(":");
            
            // Error decoding 
            if index.is_none() { return Err(ParseError) }
            let index = index.unwrap();

            let (mut key, mut value) = line.split_at(index);

            key = key.trim();
            value = value[1..value.len()].trim();

            headers.insert(key.to_lowercase(), value.to_string());
        }

        // Parse body
        let body = if body.len() == 0 { None } else { Some(body.to_string()) };

        let headers = if headers.len() == 0 { None } else { Some(headers) };
        return Ok(Response { version, status_code, status_text, headers, body });

    }
}

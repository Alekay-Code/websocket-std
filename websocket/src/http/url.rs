use regex::Regex;

const ws_url_regex: &str = r"^wss?://([a-zA-Z0-9\-._~%!$&'()*+,;=]+)(:[0-9]{1,5})?(/.*)?$";

pub fn is_valid_ws_url(url: &str) -> bool { 
    let re = Regex::new(ws_url_regex).unwrap();
    re.is_match(url)
}

pub fn is_secure(url: &str) -> bool {
   &url[0..3] == "wss"
}

// Return host and port
pub fn get_address(url: &str) -> (String, String) {
    let init = url.find("//").unwrap() + 2;
    let end = url[init..url.len()].find("/");
    let default_port = if is_secure(url) { "443" } else { "80" };

    let end = match end {
        Some(i) => i + init,
        None => url.len()
    };

    let host = &url[init..end];
    
    match host.find(":") {
        Some(i) => (host[0..i].to_string(), host[i+1..host.len()].to_string()),
        None => (host.to_string(), default_port.to_string())
    }
}

pub fn get_path(url: &str) -> &str {
    let i = url.find("//").unwrap() + 2;
    let url = &url[i..url.len()];

    match url.find("/") {
        Some(i) => &url[i..url.len()],
        None => "/"
    }

}

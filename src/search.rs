use reqwest::blocking::{Client, Response};
use std::{sync::{mpsc::{self, Receiver}, Mutex, Arc}, thread};

// Arc<Mutex<T>> -> MutexGuard<T>
macro_rules! unwrap_am {
    ($oam: expr) => {
        $oam.lock().unwrap()
    };
}

fn create_client() -> Client {
    Client::builder().danger_accept_invalid_certs(true).build().unwrap()
}

pub const MAX_DIGITS_PER_REQUEST: usize = 1000; 

pub struct Search {
    client: Arc<Mutex<Client>>,

    saved_digits: Arc<Mutex<String>>,

    preload_thread_handler: Option<thread::JoinHandle<()>>,
    search_thread_handler: Option<thread::JoinHandle<()>>,

    digits_per_request: usize, // must be <= MAX_DIGITS_PER_REQUEST
}


fn send_request(client: &Client, url: &str, query: Option<&[(&str, &str)]>) -> Result<Response, Box<dyn std::error::Error>> {
    let mut req = client.get(url);
    if let Some(query) = query {
        req = req.query(query);
    }
    Ok(req.send()?)
}

fn get_digits(client: &Client, start: usize, number_of_digits: usize) -> Result<String, Box<dyn std::error::Error>> {
    let text = send_request(&client, format!("https://api.pi.delivery/v1/pi?start={start}&numberOfDigits={number_of_digits}").as_str(), None)?.text()?;
    let digits = &text[text.find(':').unwrap() + 2 .. text.len() - 2];
    Ok(digits.to_string())
}

#[derive(PartialEq)]
pub enum SearchState {
    Idle,
    Preloading,
    Searching,
}

#[allow(dead_code)]
impl Search {
    pub fn new() -> Self {
        Self {
            client: Arc::new(Mutex::new(create_client())),
            saved_digits: Arc::default(),
            preload_thread_handler: None,
            search_thread_handler: None,
            digits_per_request: MAX_DIGITS_PER_REQUEST,
        }
    }

    pub fn get_state(&self) -> SearchState {
        if self.preload_thread_handler.is_some() {
            SearchState::Preloading
        }
        else if self.search_thread_handler.is_some() {
            SearchState::Searching
        }
        else {
            SearchState::Idle
        }
    }

    pub fn digits_loaded(&self) -> usize {
        unwrap_am!(self.saved_digits).len()
    }

    pub fn preload(&mut self, count: usize) -> (Receiver<usize>, Receiver<()>) {
        if self.get_state() != SearchState::Idle {
            panic!("Can't preload: state must be idle");
        }

        let (loa_tx, loa_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();

        let c_digits = self.saved_digits.clone();
        let c_client = self.client.clone();
        let digits_per_request = self.digits_per_request;

        self.preload_thread_handler = Some(thread::spawn(move || {
            loop {
                let len = unwrap_am!(c_digits).len();

                if loa_tx.send(len).is_err() {
                    eprintln!("Main thread is dead");
                    break;
                }
                if len >= count {
                    let _ = res_tx.send(());
                    break;
                }

                let request_digits = digits_per_request.min(count - len);
                let new_digits = get_digits(&c_client.lock().unwrap(), len, request_digits).unwrap();
                
                unwrap_am!(c_digits).push_str(new_digits.as_str());
            }
        }));

        (loa_rx, res_rx)
    }

    pub fn search(&mut self, search_for: &str) -> (Receiver<usize>, Receiver<Option<usize>>) {
        if self.get_state() != SearchState::Idle {
            panic!("Can't search: state must be idle");
        }
        
        let (pro_tx, pro_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();

        let c_digits = self.saved_digits.clone();
        let c_client = self.client.clone();
        let digits_per_request = self.digits_per_request;

        let search_for = search_for.to_string();
        self.search_thread_handler = Some(thread::spawn(move || {
            let ind = unwrap_am!(c_digits).find(search_for.as_str());
            if ind.is_some() {
                let _ = pro_tx.send(ind.unwrap());
                let _ = res_tx.send(Some(ind.unwrap()));
                return;
            }
            let mut digit = unwrap_am!(c_digits).len();
            if pro_tx.send(digit).is_err() {
                panic!("Error while sending");
            }
            
            let mut last_digits: Option<String> = None;
            loop {
                let new_digits = get_digits(&c_client.lock().unwrap(), digit, digits_per_request).unwrap();
                unwrap_am!(c_digits).push_str(new_digits.as_str());

                let mut d: String = "".to_string();
                if last_digits.is_some() {
                    d.push_str(last_digits.unwrap().as_str());
                }
                d.push_str(new_digits.as_str());

                if pro_tx.send(digit + digits_per_request * 2).is_err() {
                    panic!("Error while sending");
                }
                
                let ind = d.find(search_for.as_str());
                if ind.is_some() {
                    let _ = res_tx.send(Some(digit + ind.unwrap()));
                    break;
                }
                last_digits = Some(new_digits);

                digit += digits_per_request;
            }
        }));
        (pro_rx, res_rx)
    }

    pub fn into_idle(&mut self) {
        if self.preload_thread_handler.is_some() {
            let _ = self.preload_thread_handler.take().unwrap().join();
        }
        if self.search_thread_handler.is_some() {
            let _ = self.search_thread_handler.take().unwrap().join();
        }
    }
}

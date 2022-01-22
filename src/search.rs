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

    pub fn get_digits(&self) -> Arc<Mutex<String>> {
        self.saved_digits.clone()
    }
    pub fn digits_loaded(&self) -> usize {
        unwrap_am!(self.saved_digits).len()
    }

    pub fn preload(&mut self, count: usize, num_of_threads: usize) -> (Receiver<usize>, Receiver<()>) {
        if self.get_state() != SearchState::Idle {
            panic!("Can't preload: state must be idle");
        }
        if num_of_threads == 0 {
            panic!("Can't preload: num_of_threads must be greater then 0");
        }

        let (loa_tx, loa_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();

        let c_digits = self.saved_digits.clone();
        let c_client = self.client.clone();
        let digits_per_request = self.digits_per_request;

        self.preload_thread_handler = Some(thread::spawn(move || {
            let len = unwrap_am!(c_digits).len();
            if len >= count {
                let _ = res_tx.send(());
                return;
            }
            let per_thread = (count - len) / num_of_threads;

            // use only 1 thread
            if per_thread < digits_per_request {
                println!("Using 1 thread");

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
            }
            // use num_of_threads threads
            else {
                println!("Using {num_of_threads} threads");

                let (tloa_tx, tloa_rx) = mpsc::channel();
                let mut preload_threads_handlers = Vec::new();
                let (res_txs, res_rxs) = mpsc::channel();
    
                let gstart = unwrap_am!(c_digits).len();
                let gend = count;

                for i in 0..num_of_threads {
                    let tloa_tx = tloa_tx.clone();
    
                    let start = gstart + i * per_thread;
                    let end;
                    if i + 1 == num_of_threads {
                        end = gend;
                    }
                    else {
                        end = start + per_thread;
                    }

                    let c_client = c_client.clone();
    
                    let res_txs = res_txs.clone();
                    preload_threads_handlers.push(thread::spawn(move || {
                        let mut cur_loaded = start;
                        let mut digits = String::default();
                        loop {
                            let request_digits = digits_per_request.min(end - cur_loaded);

                            if request_digits == 0 {
                                break;
                            }

                            digits.push_str(get_digits(&c_client.lock().unwrap(), cur_loaded, request_digits).unwrap().as_str());
                            //println!("Thread {i}, request ({cur_loaded}-{}) completed", cur_loaded + request_digits);
                            cur_loaded += request_digits;

                            if tloa_tx.send(request_digits).is_err() {
                                panic!("Main preload thread is dead");
                            }
                        }
                        if res_txs.send((i, digits)).is_err() {
                            panic!("Main preload thread is dead");
                        }
                    }));
                }

                let mut result_strs = vec![String::default(); num_of_threads];
                let mut loa_len = unwrap_am!(c_digits).len();

                while result_strs.contains(&String::default()) {
                    loop {
                        let tloa_res = tloa_rx.try_recv();
                        match tloa_res {
                            Ok(add_len) => loa_len += add_len,
                            Err(err) => {
                                match err {
                                    mpsc::TryRecvError::Empty => break,
                                    mpsc::TryRecvError::Disconnected => {},
                                }
                            },
                        }
                    }
                    
                    loop {
                        let res_res = res_rxs.try_recv();
                        match res_res {
                            Ok((ind, s)) => result_strs[ind] = s,
                            Err(err) => {
                                match err {
                                    mpsc::TryRecvError::Empty => break,
                                    mpsc::TryRecvError::Disconnected => {},
                                }
                            },
                        }
                    }

                    if loa_tx.send(loa_len).is_err() {
                        panic!("Main thread is dead");
                    }
                }

                for rs in result_strs {
                    unwrap_am!(c_digits).push_str(rs.as_str());
                }

                let _ = res_tx.send(());
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
                panic!("Main thread is dead");
            }
            
            let mut last_digits: Option<String> = None;
            loop {
                let new_digits = get_digits(&c_client.lock().unwrap(), digit, digits_per_request).unwrap();
                unwrap_am!(c_digits).push_str(new_digits.as_str());

                let mut d: String = "".to_string();
                if last_digits.is_some() {
                    d.push_str(last_digits.as_ref().unwrap().as_str());
                }
                d.push_str(new_digits.as_str());

                if pro_tx.send(digit + digits_per_request).is_err() {
                    panic!("Main thread is dead");
                }
                
                let ind = d.find(search_for.as_str());
                if ind.is_some() {
                    let _ = res_tx.send(Some(digit + ind.unwrap() - last_digits.as_ref().unwrap_or(&"".to_string()).len()));
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

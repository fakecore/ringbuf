mod core;

#[cfg(test)]
mod tests {
    use std::process::exit;
    use crate::core::{MsgQueue};
    use super::*;
    #[test]
    fn it_works() {
        let mut msg_queue = MsgQueue::new();
        msg_queue.set_subscription("hi".to_string());
        let control = match msg_queue.get_subscription("hi".to_string()){
            Ok(control) =>{ control },
            Err(str) => {panic!("err:{}",str)}
        };
        control.print_hello();
        println!("subscription-name:{}",control.subscription_name());
        println!("exist:{}",control.is_exist());
    }

    #[test]
    fn test_msg_queue() {
        let mut msg_queue = MsgQueue::new();
        msg_queue.set_subscription("hi".to_string());
        let mut control = match msg_queue.get_subscription("hi".to_string()){
            Ok(control) =>{ control },
            Err(str) => {panic!("err:{}",str)}
        };
        control.push_data(vec![0;1000]);
        assert_eq!(control.size(),1000);
        let data = control.read_all();
        assert_eq!(control.size(),0);
        assert_eq!(data.len(),1000);
        let data = control.read(1000);
        assert_eq!(data.len(),0);
        // block_on(control.readable().await);
    }

}

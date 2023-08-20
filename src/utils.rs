use rand::distributions::Alphanumeric;
use rand::Rng;

pub fn get_random_string(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

#[test]
fn create_sample_notification_test(){
    let res = create_sample_notification("hello", "world", Some("yes,no".to_string()));
    println!("{}", res)
}

pub fn create_sample_notification(title: &str, message: &str, buttons: Option<String>) -> String {
    let mut actions = String::new();
    if let Some(btn) = buttons{
        let actions_list: Vec<String> = btn.split(',').map(|b|format!("<action arguments=\"{}\" content=\"{}\"/>", b, b).to_string()).collect();
        if actions_list.len() > 0 {
            actions.push_str("<actions>");
            actions.push_str( actions_list.join("\n").as_str());
            actions.push_str("</actions>");
        }
    }
    let string = format!("
<toast>
  <visual>
    <binding template=\"ToastGeneric\">
      <text>{}</text>
      <text>{}</text>
    </binding>
  </visual>
    {}
</toast>", title, message, actions).to_string();
    string
}
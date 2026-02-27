use text_fragments::*;

/**
For `1 Corinthians 12:27`: `members%20of%20it.-,1%20Corinthians%2012%3A27,-(ESV)`

URL: `https://svrbc.org/articles/2020-12-21/is-church-membership-biblical/#:~:text=members%20of%20it.-,1%20Corinthians%2012%3A27,-(ESV)`
*/
#[test]
fn test1() {
    let html = include_str!("./html/Is Church Membership Biblical? - by SVRBC.html");
    let doc = Document::from_html(html);
    let range = Selection {
        start: Position {
            line: 59,
            column: 25,
        },
        end: Position {
            line: 59,
            column: 44,
        },
    };
    let doc = FragmentGenerator::new(doc);
    let tf = doc.generate(range).unwrap();
    dbg!(&tf);
    dbg!(&tf.to_hash_string());
}

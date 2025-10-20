use anyhow::bail;

use std::{
    io::{self, Write},
    thread,
    time::Duration,
};

use terminal_size::{terminal_size, Height, Width};

fn clear_screen() {
    print!("\x1B[2J\x1B[1;1H");
}

#[allow(clippy::too_many_lines)]
pub fn cmd() -> anyhow::Result<()> {
    if let Some((Width(w), Height(h))) = terminal_size() {
        if w < 180 || h < 40 {
            bail!("Window size is too small, are you working from PSP? Set it at least for 120x40 symbols (≈800x800).");
        }
    }

    let bike1 = r"
                                                 (|
                                                   ||_
                                                  =///`\   
                             (\                   \\\) | 
                            __\\                   `|~~|    
                           (((<_|            ____   |  |     
                            `-__/\         /~    ~\|   |    
                               \  ~-_     |--|     |___|    
                                `\   ~-_  |_/     /--__/   
                                  `\/ / ~-_\___--/    /   
                                    `-_    ~/   /    /   
                                       ~-_ /   |   _/   
                                          |         |
                                         |~~~~~-----| 
                                         |___----~~~/
              _-~~\                       \_       /
            /(_|_-~                       |       /                          
          /   /~==[]\     ____-------_    |_____--|   ______________       
        /    (_ //(\0)~~~~     BMW    ~\ /_-       \/'         ___/ ~~~~/  
       (|      ~~--__                   |/         )_____---~~~    YZF  \    
        \.      ___  ~~--__ ____        /        _-/              __--~~'  
          ~\    \\\\       ~~-_ ~-____ /      _-~~          __--~~___ 
     _ ----/ \    \\\\         ~-_    /---__-~        __--~~----~~_  ]=
  _-~ ___ / /__\   ~~~            ~-_ ( )-~ ~-_~~~/~~~ _-~         ~-_ 
 /-~~~_-|/ /    ~\                  _) ~-_     \ /~~~~~---__-----_    \
;    / \/_//`\    \           __--~~/_   \~-_ _-\ ~~~~~~~~~~~~-/_/\    .
|   | \((*))/ |   |\    __--~~     /o \   `\ ~-  `\----_____( 0) ) |   | 
|    \  |~|  /    | )-~~           \ 0 )    |/' _-~/~--------| |~ /    ,
 \    ~-----~    / /                ~~~~~~~~/_/O_/'   \    ~-----~    /
  ~-_         _-~ `---------------------------'        `-_         _-~
     ~ ----- ~                                            ~ ----- ~ 
";

    let bike2 = r"







                            ___
                          /~   ~\
                         |_      |
                         |/     __-__
                          \   /~     ~~-_
                           ~~ -~~\       ~\
                            /     |        \
               ,           /     /          \
             //   _ _---~~~    //-_          \
           /  (/~~ )    _____/-__  ~-_       _-\             _________
         /  _-~\\0) ~~~~         ~~-_ \__--~~   `\  ___---~~~        /'
        /_-~               BMW     _-/'          )~/               /'
        (___________/           _-~/'         _-~~/             _-~
     _ ----- _~-_\\\\        _-~ /'      __--~   (_ ______---~~~--_
  _-~         ~-_~\\\\      (   (     -_~          ~-_  |          ~-_
 /~~~~\          \ \~~       ~-_ ~-_    ~\            ~~--__-----_    \
;    / \ ______-----\           ~-__~-~~~~~~--_             ~~--_ \    .
|   | \((*)~~~~~~~~~~|      __--~~             ~-_               ) |   |
|    \  |~|~---------)__--~~                      \_____________/ /    ,
 \    ~-----~    /  /~                             )  \    ~-----~    /
  ~-_         _-~ /_______________________________/    `-_         _-~
     ~ ----- ~                                            ~ ----- ~ 
";

    let biker_stop = r"


                        ____
                       / -- \
                       ||__||
                       |____|
                     ___)  (___
                    XXXXX  XXXXX
                   XX (XXXXXX) XX
                  XX   XXXXXX   XX
                   XX   XXXX     XX
                    XX  HHHH      XX
                     M HHHHHH       M   ___
                      HHH  HHH       ==|___|==
                      HHH  HHH         \___/
                      HHH  HHH      O--(( ))--O
                      HHH  HHH         \nnn/
                      HHH  HHH         n   n
                      HHH  HHH          /O\
                      HHH  HHH          OOO
                      HHH  HHH          OOO
                      VVV  VVV          OOO
                      |V/  \V/          OOO
                     /_/|  |\_\         \O/
";
    let biker_no_more = r"













                                     ==|___|==
                                       \___/
                                    O--(( ))--O
                                       \nnn/
                                       n   n
                                        /O\
                                        OOO
                                        OOO
                                        OOO
                                        OOO
                                        \O/

Hello :)
My name is Eugene, I am the author of Pike.
If you stumbled upon this command by accident, you might have been overworking lately,
I strongly recommend you to rest well and don't work on weekends :) 
Make your life a ride and enjoy the life, which, alas, not infinite.
This is my final message, farewell.
";

    let width = 100;
    for pos in (0..width).rev() {
        clear_screen();
        let sprite = if pos > width / 2 { bike1 } else { bike2 };
        for line in sprite.lines() {
            println!("{:>width$}", line, width = pos + line.len());
        }
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_millis(80));
    }

    clear_screen();
    println!("{biker_stop}");
    io::stdout().flush().unwrap();

    let phrases = [
        "Hello :)",
        "My name is Eugene, I am the author of Pike.",
        "If you stumbled upon this command by accident, you might have been overworking lately,",
        "I strongly recommend you to rest well and don't work on weekends :) ",
        "Make your life a ride and enjoy the life, which, alas, not infinite.",
        "This is my final message, farewell.",
    ];

    for phrase in phrases {
        println!("{phrase}");
        thread::sleep(Duration::from_secs(3));
    }

    clear_screen();
    println!("{biker_no_more}");
    thread::sleep(Duration::from_secs(3));

    io::stdout().flush().unwrap();

    Ok(())
}

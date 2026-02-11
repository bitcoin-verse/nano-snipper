//! About page: Bitcoin.com sponsor information.

use crate::theme;
use iced::widget::{column, container, scrollable, text, Space};
use iced::{Element, Length};

#[derive(Debug, Clone)]
pub enum Message {
    OpenBitcoinCom,
}

pub fn view<'a>() -> Element<'a, Message> {
    let header = column![
        iced::widget::image(iced::widget::image::Handle::from_bytes(
            include_bytes!("../../../../resources/bitcoincom_logo_light.png").as_slice(),
        ))
        .width(180),
        Space::with_height(8),
        text("We believe in a world where financial freedom is accessible to everyone. Bitcoin.com is building the tools to make that vision a reality.")
            .size(13)
            .color(theme::TEXT_SECONDARY),
    ]
    .spacing(4);

    let header_card = container(header)
        .style(theme::card)
        .padding(20)
        .width(Length::Fill);

    let mission_card = container(
        column![
            text("Our mission is to create more economic freedom in the world")
                .size(16)
                .font(theme::FONT_SEMIBOLD)
                .color(theme::TEXT_PRIMARY),
            text(
                "We define economic freedom as the ability to make choices with respect \
                to one's personal resources, unencumbered by trusted third parties or \
                borders or lack of access. We believe economic freedom is the foundation \
                of peace and prosperity, and by creating more of it for people, we are \
                reducing suffering in the world. Our mission is to help everyone, \
                everywhere be more economically free."
            )
            .size(13)
            .color(theme::TEXT_SECONDARY),
            Space::with_height(4),
            pillar("We empower people to fully own and freely manage their finances through self-custody."),
            pillar("We educate people about their rights to financial independence, ownership, and privacy."),
            pillar("We provide people worldwide with access to a borderless, decentralized financial system."),
        ]
        .spacing(8),
    )
    .style(theme::card)
    .padding(16)
    .width(Length::Fill);

    let what_card = container(
        column![
            text("What we're doing about it")
                .size(16)
                .font(theme::FONT_SEMIBOLD)
                .color(theme::TEXT_PRIMARY),
            text(
                "Since 2015, Bitcoin.com has been a global leader in introducing \
                newcomers to crypto. We make it easy for anyone to buy, spend, trade, \
                invest, earn, and stay up-to-date on cryptocurrency and the future of finance."
            )
            .size(13)
            .color(theme::TEXT_SECONDARY),
            Space::with_height(4),
            product(
                "Wallet App",
                "Use the multichain mobile wallet app trusted by millions to buy, sell, trade, earn, \
                use, and learn about Bitcoin and other cryptocurrencies. Every feature you need for \
                economic freedom in one self-custodial digital wallet."
            ),
            product(
                "Bitcoin.com Account",
                "Your Bitcoin.com account enables you to manage your cryptocurrency from any device \
                or browser. Use the intuitive dashboard to buy, sell, and trade crypto wherever you \
                are. Protected by the cutting-edge MPC technology, it combines great user experience \
                with robust security."
            ),
            product(
                "Verse DEX",
                "Trade cryptocurrencies anonymously, securely, and with low fees using Bitcoin.com's \
                decentralized exchange Verse DEX. Earn yield by providing liquidity to the exchange, \
                depositing tokens in Verse Farms, and by staking VERSE."
            ),
            product(
                "News",
                "Stay informed with timely and objective news content relevant to the crypto industry, \
                published daily by Bitcoin.com News. Access price forecasts, expert analytics, and \
                exclusive insights into cryptocurrency, blockchain, regulation, finance, economics, \
                and beyond."
            ),
            product(
                "Learning Center",
                "Get educated with up-to-date content from Bitcoin.com's learning center that spans \
                the basic value proposition of crypto and how to safely acquire, hold, and use it, \
                to the technologies, applications and market insights most relevant to enthusiasts."
            ),
        ]
        .spacing(8),
    )
    .style(theme::card)
    .padding(16)
    .width(Length::Fill);

    let trust_card = container(
        column![
            text("Trusted & Verified")
                .size(16)
                .font(theme::FONT_SEMIBOLD)
                .color(theme::TEXT_PRIMARY),
            text(
                "We are proud to be ISO 27001 certified, affirming our commitment to the \
                highest standards of information security management to protect both our \
                data and that of our clients, ensuring trust and opening doors to \
                international opportunities."
            )
            .size(13)
            .color(theme::TEXT_SECONDARY),
        ]
        .spacing(8),
    )
    .style(theme::card)
    .padding(16)
    .width(Length::Fill);

    let link_btn = iced::widget::button(
        text("Visit www.bitcoin.com")
            .size(13)
            .color(theme::TEXT_PRIMARY),
    )
    .on_press(Message::OpenBitcoinCom)
    .style(theme::ghost_button)
    .padding([8, 16]);

    let content = scrollable(
        column![
            header_card,
            mission_card,
            what_card,
            trust_card,
            link_btn,
        ]
        .spacing(16)
        .padding(24)
        .width(Length::Fill),
    )
    .direction(iced::widget::scrollable::Direction::Vertical(
        iced::widget::scrollable::Scrollbar::new().width(4).scroller_width(4),
    ))
    .style(theme::thin_scrollbar)
    .height(Length::Fill);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn pillar<'a>(desc: &'a str) -> Element<'a, Message> {
    text(format!("  •  {desc}"))
        .size(13)
        .color(theme::TEXT_PRIMARY)
        .into()
}

fn product<'a>(title: &'a str, desc: &'a str) -> Element<'a, Message> {
    column![
        text(title)
            .size(13)
            .font(theme::FONT_SEMIBOLD)
            .color(theme::TEXT_PRIMARY),
        text(desc).size(12).color(theme::TEXT_TERTIARY),
    ]
    .spacing(2)
    .into()
}

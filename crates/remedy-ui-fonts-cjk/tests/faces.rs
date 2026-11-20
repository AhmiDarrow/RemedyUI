//! The Chinese and Japanese faces parse, are what they say, and cover what they promise.

use read_fonts::{FontRef, TableProvider};

/// Every character two bytes of `codec`-style EUC rows give (GB 2312, JIS X 0208 level 1),
/// sampled: the most common hanzi and kanji, kana and punctuation.
const COMMON_ZH: &str = "的一是不了人我在有他这中大来上国个到说们为子和你地出道也时年得就那要下以生会自着去之过家学对可她里后小么心多天而能好都然没日于起还发成事只作当想看文无开手十用主行方又如前所本见经头面公同三已老从动两长知民样现分将外但身些与高意进把法此实回二理美点月明其种声全工己话儿者向情部正名定女问力机给等几很业最间新什打便位因重被走电四第门相次东政海口使教西再平真听世气信北少关并内加化由却代军产入先山五太水万市眼体别处总才场师书比住员九笑性通目华报立马命张活难神数件安表原车白应路期叫死常提感金何更反合放做系计或司利受光王果亲界及今京务制解各任至清物台象记边共风战干接它许八特觉望直服毛林题建南度统色字请交爱让认算论百吃义科怎元社术结六功指思非流每青管夫连远资队跟带花快条院变联言权往展该领传近留红治决周保达办运武半候七必城父强步完革深区即求品士转量空甚众技轻程告江语英基派满式李息写呢识极令黄德收脸钱党倒未持取设始版双历越史商千片容研像找友孩站广改议形委早房音火际则首单据导影失拿网香似斯专石若兵弟谁校读志飞观争究包组造落视济喜离虽坐集编宝谈府拉黑且随格尽剑讲布杀微怕母调局根曾准团段终乐切级克精哪官示冷域读层";
const COMMON_JA: &str = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをんがぎぐげござじずぜぞだぢづでどばびぶべぼぱぴぷぺぽゃゅょっアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲンヴー・「」『』、。日本語漢字東京大阪北海道沖縄学校先生会社電話時間今日明日昨日月火水木金土曜年週朝昼夜春夏秋冬山川海空雨雪風花鳥魚犬猫人子女男父母兄弟姉妹友達家族愛心手足目耳口顔体頭髪色赤青黄白黒茶緑紫桃食飲物事場所部屋窓戸机椅子本紙書読聞見言話思考知分感動作使行来帰入出始終開閉買売払借貸送届決定変更保存削除編集表示設定検索選択確認取消終了";

#[test]
fn every_face_parses_and_covers_its_language() -> Result<(), Box<dyn std::error::Error>> {
    let faces = remedy_ui_fonts_cjk::all();
    assert_eq!(faces.len(), 4, "Chinese and Japanese, Regular and Bold");
    for face in faces {
        let sfnt = wuff::decompress_woff2(face.data).map_err(|e| format!("{e:?}"))?;
        let font = FontRef::new(&sfnt)?;
        assert_eq!(font.os2()?.us_weight_class(), face.weight, "{}", face.family);
        assert_eq!(font.os2()?.fs_type(), 0, "{} is installable and embeddable", face.family);
        let name = font.name()?;
        let family = name
            .name_record()
            .iter()
            .find(|r| r.name_id().to_u16() == 1)
            .and_then(|r| r.string(name.string_data()).ok())
            .map(|s| s.chars().collect::<String>());
        assert_eq!(family.as_deref(), Some(face.family));
        let cmap = font.cmap()?;
        let sample = if face.family.ends_with("SC") { COMMON_ZH } else { COMMON_JA };
        for c in sample.chars().chain(' '..='~') {
            assert!(
                cmap.map_codepoint(c).is_some_and(|g| g.to_u32() != 0),
                "{} {} lacks {c:?}",
                face.family,
                face.weight
            );
            assert!(c.is_ascii() || remedy_ui_fonts_cjk::needs(c), "{c:?} would not load the faces");
        }
    }
    Ok(())
}

#[test]
fn han_falls_back_by_language_and_kana_to_japanese() {
    let fb = remedy_ui_fonts_cjk::script_fallbacks();
    let first = |script: &str, locale: Option<&str>| {
        fb.iter().find(|f| f.script == script && f.locale == locale).map(|f| f.family)
    };
    assert_eq!(first("Hani", Some("ja")), Some("Noto Sans JP"));
    assert_eq!(first("Hani", Some("zh-CN")), Some("Noto Sans SC"));
    assert_eq!(first("Hani", None), Some("Noto Sans SC"));
    assert_eq!(first("Hira", None), Some("Noto Sans JP"));
    assert_eq!(first("Kana", None), Some("Noto Sans JP"));
    assert!(!remedy_ui_fonts_cjk::needs('a') && !remedy_ui_fonts_cjk::needs('م'));
}

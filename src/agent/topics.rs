//! Research-topic vocabulary used by the recommender.
//!
//! User interests are free text in English or Chinese ("多模态大模型",
//! "program analysis"). They are mapped onto a small set of canonical topic
//! tags, each of which knows the CCF categories it belongs to. Venues get tags
//! from a curated hint table plus any English alias that appears in their
//! full name, so even conferences without a hint are matched sensibly.

use std::collections::BTreeSet;

/// A canonical research topic.
pub struct Topic {
    pub tag: &'static str,
    pub name_en: &'static str,
    pub name_zh: &'static str,
    /// CCF categories (`sub` codes) this topic usually publishes in.
    pub subs: &'static [&'static str],
    /// Lower-case aliases matched as substrings of user text.
    pub aliases: &'static [&'static str],
    /// Aliases that are specific enough to match a venue's full name.
    pub venue_aliases: &'static [&'static str],
}

pub const TOPICS: &[Topic] = &[
    Topic {
        tag: "cv",
        name_en: "computer vision",
        name_zh: "计算机视觉",
        subs: &["AI", "CG"],
        aliases: &[
            "computer vision",
            "vision",
            "视觉",
            "图像",
            "image",
            "video understanding",
            "视频理解",
            "object detection",
            "目标检测",
            "segmentation",
            "分割",
            "cv",
        ],
        venue_aliases: &["vision", "image", "pattern recognition"],
    },
    Topic {
        tag: "nlp",
        name_en: "natural language processing",
        name_zh: "自然语言处理",
        subs: &["AI"],
        aliases: &[
            "nlp",
            "natural language",
            "自然语言",
            "text",
            "文本",
            "machine translation",
            "机器翻译",
            "dialogue",
            "对话",
            "information extraction",
            "信息抽取",
        ],
        venue_aliases: &[
            "computational linguistics",
            "natural language",
            "language processing",
        ],
    },
    Topic {
        tag: "llm",
        name_en: "large language models",
        name_zh: "大语言模型",
        subs: &["AI"],
        aliases: &[
            "llm",
            "large language model",
            "language model",
            "大模型",
            "大语言模型",
            "语言模型",
            "gpt",
            "foundation model",
            "基础模型",
            "prompt",
            "rag",
        ],
        venue_aliases: &["language model"],
    },
    Topic {
        tag: "multimodal",
        name_en: "multimodal learning",
        name_zh: "多模态",
        subs: &["AI", "CG"],
        aliases: &[
            "multimodal",
            "multi-modal",
            "多模态",
            "vision-language",
            "vision language",
            "视觉语言",
            "图文",
            "mllm",
            "vlm",
        ],
        venue_aliases: &["multimedia"],
    },
    Topic {
        tag: "ml",
        name_en: "machine learning",
        name_zh: "机器学习",
        subs: &["AI"],
        aliases: &[
            "machine learning",
            "机器学习",
            "deep learning",
            "深度学习",
            "reinforcement learning",
            "强化学习",
            "representation learning",
            "表示学习",
            "federated learning",
            "联邦学习",
            "learning theory",
            "神经网络",
            "neural network",
        ],
        venue_aliases: &[
            "machine learning",
            "neural information",
            "learning representations",
        ],
    },
    Topic {
        tag: "ai",
        name_en: "artificial intelligence",
        name_zh: "人工智能",
        subs: &["AI"],
        aliases: &[
            "artificial intelligence",
            "人工智能",
            "knowledge representation",
            "知识表示",
            "planning",
            "规划",
            "reasoning",
            "推理能力",
            "agent",
            "智能体",
        ],
        venue_aliases: &["artificial intelligence", "agent"],
    },
    Topic {
        tag: "robotics",
        name_en: "robotics & embodied AI",
        name_zh: "机器人与具身智能",
        subs: &["AI"],
        aliases: &[
            "robot",
            "机器人",
            "embodied",
            "具身",
            "autonomous driving",
            "自动驾驶",
        ],
        venue_aliases: &["robot"],
    },
    Topic {
        tag: "speech",
        name_en: "speech & audio",
        name_zh: "语音与音频",
        subs: &["AI", "CG"],
        aliases: &["speech", "语音", "audio", "音频", "asr", "tts"],
        venue_aliases: &["speech", "acoustics", "signal processing"],
    },
    Topic {
        tag: "datamining",
        name_en: "data mining",
        name_zh: "数据挖掘",
        subs: &["DB"],
        aliases: &[
            "data mining",
            "数据挖掘",
            "graph neural",
            "图神经",
            "gnn",
            "time series",
            "时序",
            "spatio-temporal",
            "时空",
            "knowledge graph",
            "知识图谱",
            "big data",
            "大数据",
        ],
        venue_aliases: &[
            "data mining",
            "knowledge discovery",
            "knowledge management",
            "big data",
        ],
    },
    Topic {
        tag: "ir",
        name_en: "search & recommendation",
        name_zh: "搜索与推荐",
        subs: &["DB"],
        aliases: &[
            "recommend",
            "推荐",
            "information retrieval",
            "信息检索",
            "检索",
            "retrieval",
            "search engine",
            "搜索",
            "web search",
        ],
        venue_aliases: &["retrieval", "recommender", "web search", "world wide web"],
    },
    Topic {
        tag: "database",
        name_en: "databases & data management",
        name_zh: "数据库与数据管理",
        subs: &["DB"],
        aliases: &[
            "database",
            "数据库",
            "query",
            "查询",
            "sql",
            "transaction",
            "事务",
            "data management",
            "数据管理",
            "data lake",
        ],
        venue_aliases: &[
            "database",
            "data management",
            "data engineering",
            "large data bases",
        ],
    },
    Topic {
        tag: "security",
        name_en: "security & privacy",
        name_zh: "安全与隐私",
        subs: &["SC"],
        aliases: &[
            "security",
            "安全",
            "privacy",
            "隐私",
            "attack",
            "攻击",
            "adversarial",
            "对抗",
            "malware",
            "恶意",
            "fuzzing",
            "模糊测试",
            "cryptograph",
            "密码",
            "blockchain",
            "区块链",
            "漏洞",
            "vulnerability",
        ],
        venue_aliases: &[
            "security",
            "privacy",
            "cryptolog",
            "cryptograph",
            "encryption",
        ],
    },
    Topic {
        tag: "network",
        name_en: "computer networks",
        name_zh: "计算机网络",
        subs: &["NW"],
        aliases: &[
            "computer network",
            "计算机网络",
            "networking",
            "wireless",
            "无线",
            "5g",
            "6g",
            "sdn",
            "edge computing",
            "边缘计算",
            "iot",
            "物联网",
            "internet of things",
            "mobile computing",
            "移动计算",
            "sensor network",
            "传感网",
            "network measurement",
            "网络测量",
        ],
        venue_aliases: &["network", "communication", "mobile", "sensor"],
    },
    Topic {
        tag: "systems",
        name_en: "systems & distributed computing",
        name_zh: "系统与分布式计算",
        subs: &["DS"],
        aliases: &[
            "distributed",
            "分布式",
            "operating system",
            "操作系统",
            "cloud",
            "云计算",
            "serverless",
            "storage",
            "存储",
            "file system",
            "文件系统",
            "gpu",
            "parallel",
            "并行",
            "hpc",
            "高性能计算",
            "llm serving",
            "推理加速",
            "训练系统",
            "mlsys",
        ],
        venue_aliases: &[
            "operating system",
            "distributed",
            "parallel",
            "storage",
            "file",
            "cloud",
            "high performance",
            "supercomputing",
            "cluster",
        ],
    },
    Topic {
        tag: "architecture",
        name_en: "computer architecture & hardware",
        name_zh: "体系结构与硬件",
        subs: &["DS"],
        aliases: &[
            "computer architecture",
            "体系结构",
            "accelerator",
            "加速器",
            "芯片",
            "chip",
            "fpga",
            "hardware",
            "硬件",
            "eda",
            "vlsi",
        ],
        venue_aliases: &[
            "architecture",
            "design automation",
            "circuit",
            "vlsi",
            "field-programmable",
            "fpga",
            "hardware",
        ],
    },
    Topic {
        tag: "se",
        name_en: "software engineering",
        name_zh: "软件工程",
        subs: &["SE"],
        aliases: &[
            "software engineering",
            "软件工程",
            "software testing",
            "软件测试",
            "program analysis",
            "程序分析",
            "code generation",
            "代码生成",
            "代码",
            "program repair",
            "程序修复",
            "缺陷",
            "bug",
            "mining software",
        ],
        venue_aliases: &["software", "testing", "maintenance", "requirements"],
    },
    Topic {
        tag: "pl",
        name_en: "programming languages & verification",
        name_zh: "程序设计语言与形式化验证",
        subs: &["SE", "CT"],
        aliases: &[
            "programming language",
            "编程语言",
            "程序设计语言",
            "compiler",
            "编译",
            "type system",
            "类型系统",
            "formal verification",
            "形式化",
            "model checking",
            "模型检测",
        ],
        venue_aliases: &[
            "programming",
            "compil",
            "verification",
            "formal methods",
            "logic",
        ],
    },
    Topic {
        tag: "theory",
        name_en: "theory of computing",
        name_zh: "计算理论",
        subs: &["CT"],
        aliases: &[
            "theory of computing",
            "计算理论",
            "algorithm theory",
            "算法理论",
            "complexity",
            "复杂性",
            "approximation algorithm",
            "近似算法",
            "graph theory",
            "图论",
            "combinatorial",
            "组合优化",
            "information theory",
            "信息论",
        ],
        venue_aliases: &[
            "theory of computing",
            "algorithms",
            "computational geometry",
            "automata",
        ],
    },
    Topic {
        tag: "graphics",
        name_en: "graphics & visualization",
        name_zh: "图形学与可视化",
        subs: &["CG"],
        aliases: &[
            "graphics",
            "图形学",
            "rendering",
            "渲染",
            "animation",
            "动画",
            "geometry processing",
            "几何处理",
            "nerf",
            "gaussian splatting",
            "3d reconstruction",
            "三维重建",
            "virtual reality",
            "虚拟现实",
            "vr",
            "ar",
            "visualization",
            "可视化",
        ],
        venue_aliases: &[
            "graphics",
            "visualization",
            "virtual reality",
            "geometry",
            "animation",
        ],
    },
    Topic {
        tag: "multimedia",
        name_en: "multimedia",
        name_zh: "多媒体",
        subs: &["CG"],
        aliases: &[
            "multimedia",
            "多媒体",
            "video",
            "视频",
            "image processing",
            "图像处理",
        ],
        venue_aliases: &["multimedia", "image processing", "video"],
    },
    Topic {
        tag: "hci",
        name_en: "human-computer interaction",
        name_zh: "人机交互",
        subs: &["HI"],
        aliases: &[
            "hci",
            "human-computer",
            "human computer",
            "人机交互",
            "user study",
            "用户研究",
            "accessibility",
            "无障碍",
            "ubiquitous",
            "普适计算",
            "wearable",
            "可穿戴",
            "crowdsourcing",
            "众包",
            "cscw",
        ],
        venue_aliases: &[
            "human factors",
            "human-computer",
            "interaction",
            "user interface",
            "ubiquitous",
            "pervasive",
            "cooperative work",
        ],
    },
    Topic {
        tag: "bio",
        name_en: "bioinformatics & medical AI",
        name_zh: "生物信息与医疗智能",
        subs: &["MX"],
        aliases: &[
            "bioinformatics",
            "生物信息",
            "medical",
            "医学",
            "医疗",
            "healthcare",
            "健康",
            "protein",
            "蛋白",
            "molecular",
            "分子",
            "drug",
            "药物",
        ],
        venue_aliases: &[
            "bioinformatics",
            "biology",
            "medical",
            "molecular",
            "biomedicine",
        ],
    },
];

/// Hand-curated tags for well-known venues. Full venue names are matched as
/// well (see [`venue_tags`]), so this only needs to cover venues whose names do
/// not reveal their focus.
const VENUE_HINTS: &[(&str, &[&str])] = &[
    ("AAAI", &["ai", "ml", "nlp", "cv", "llm", "multimodal"]),
    ("IJCAI", &["ai", "ml", "nlp", "cv", "llm"]),
    ("NeurIPS", &["ml", "ai", "llm", "multimodal", "cv"]),
    ("ICML", &["ml", "ai", "llm"]),
    ("ICLR", &["ml", "llm", "cv", "multimodal"]),
    ("COLT", &["ml", "theory"]),
    ("UAI", &["ml", "ai"]),
    ("AISTATS", &["ml"]),
    ("CVPR", &["cv", "multimodal", "graphics"]),
    ("ICCV", &["cv", "multimodal", "graphics"]),
    ("ECCV", &["cv", "multimodal"]),
    ("ACL", &["nlp", "llm", "multimodal"]),
    ("EMNLP", &["nlp", "llm", "multimodal"]),
    ("NAACL", &["nlp", "llm"]),
    ("COLING", &["nlp", "llm"]),
    ("COLM", &["llm", "nlp"]),
    ("CoNLL", &["nlp"]),
    ("ICRA", &["robotics", "cv"]),
    ("IROS", &["robotics", "cv"]),
    ("RSS", &["robotics", "ml"]),
    ("CoRL", &["robotics", "ml"]),
    ("AAMAS", &["ai"]),
    ("ACM MM", &["multimedia", "multimodal", "cv", "speech"]),
    ("ICME", &["multimedia", "multimodal", "cv"]),
    ("ICMR", &["multimedia", "ir", "multimodal"]),
    ("ACM SIGGRAPH", &["graphics", "cv"]),
    ("ACM SIGGRAPH ASIA", &["graphics", "cv"]),
    ("IEEE VIS", &["graphics", "hci"]),
    ("IEEE VR", &["graphics", "hci"]),
    ("ICASSP", &["speech", "multimedia"]),
    ("InterSpeech", &["speech", "nlp"]),
    ("SIGKDD", &["datamining", "ml", "ir"]),
    ("SIGIR", &["ir", "llm", "nlp"]),
    ("WWW", &["ir", "datamining", "llm"]),
    ("WSDM", &["ir", "datamining"]),
    ("CIKM", &["ir", "datamining", "database"]),
    ("RecSys", &["ir"]),
    ("ICDM", &["datamining", "ml"]),
    ("SIGMOD", &["database", "systems"]),
    ("VLDB", &["database", "systems"]),
    ("ICDE", &["database", "datamining"]),
    ("CCS", &["security"]),
    ("S&P", &["security"]),
    ("USENIX Security", &["security"]),
    ("NDSS", &["security", "network"]),
    ("ICSE", &["se", "llm"]),
    ("FSE", &["se", "llm"]),
    ("ASE", &["se", "llm"]),
    ("ISSTA", &["se", "pl"]),
    ("PLDI", &["pl"]),
    ("POPL", &["pl", "theory"]),
    ("OOPSLA", &["pl", "se"]),
    ("OSDI", &["systems"]),
    ("SOSP", &["systems"]),
    ("EuroSys", &["systems"]),
    ("SIGOPS ATC", &["systems"]),
    ("ASPLOS", &["architecture", "systems", "pl"]),
    ("ISCA", &["architecture"]),
    ("MICRO", &["architecture"]),
    ("HPCA", &["architecture"]),
    ("MLSys", &["systems", "ml", "llm"]),
    ("SIGCOMM", &["network"]),
    ("NSDI", &["network", "systems"]),
    ("MobiCom", &["network", "hci"]),
    ("INFOCOM", &["network"]),
    ("CHI", &["hci"]),
    ("UIST", &["hci"]),
    ("CSCW", &["hci"]),
    ("UbiComp/ISWC", &["hci", "network"]),
    ("STOC", &["theory"]),
    ("FOCS", &["theory"]),
    ("SODA", &["theory"]),
    ("MICCAI", &["bio", "cv"]),
    ("ISMB", &["bio"]),
    ("RECOMB", &["bio"]),
];

pub fn topic(tag: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|topic| topic.tag == tag)
}

/// Map free text (any language) to topic tags.
pub fn tags_for_text(text: &str) -> BTreeSet<&'static str> {
    let lower = text.to_lowercase();
    TOPICS
        .iter()
        .filter(|topic| {
            topic
                .aliases
                .iter()
                .any(|alias| contains_alias(&lower, alias))
        })
        .map(|topic| topic.tag)
        .collect()
}

/// Topic tags for a venue: curated hints plus aliases found in its full name.
pub fn venue_tags(title: &str, description: &str) -> BTreeSet<&'static str> {
    let mut tags: BTreeSet<&'static str> = VENUE_HINTS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(title))
        .map(|(_, tags)| tags.iter().copied().collect())
        .unwrap_or_default();
    // "Neural Networks" in a venue name says nothing about computer networks.
    let lower = description.to_lowercase().replace("neural network", "");
    for topic in TOPICS {
        if topic
            .venue_aliases
            .iter()
            .any(|alias| contains_alias(&lower, alias))
        {
            tags.insert(topic.tag);
        }
    }
    tags
}

/// `true` when the venue's focus comes from the curated table, so a missing
/// overlap is meaningful rather than a gap in our knowledge.
pub fn has_curated_hint(title: &str) -> bool {
    VENUE_HINTS
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(title))
}

/// Short ASCII aliases ("cv", "ar", "vr", "5g") must match whole words, so
/// "ar" does not fire inside "search" and "cv" not inside "cvpr".
fn contains_alias(haystack: &str, alias: &str) -> bool {
    let short_ascii = alias.len() <= 4 && alias.is_ascii();
    if !short_ascii {
        return haystack.contains(alias);
    }
    let bytes = haystack.as_bytes();
    let mut start = 0;
    while let Some(found) = haystack[start..].find(alias) {
        let begin = start + found;
        let end = begin + alias.len();
        let before_ok = begin == 0 || !bytes[begin - 1].is_ascii_alphanumeric();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = begin + 1;
        while !haystack.is_char_boundary(start) {
            start += 1;
        }
    }
    false
}

/// CCF categories implied by a set of tags.
pub fn subs_for_tags<'a>(tags: impl IntoIterator<Item = &'a &'static str>) -> BTreeSet<String> {
    tags.into_iter()
        .filter_map(|tag| topic(tag))
        .flat_map(|topic| topic.subs.iter().map(|sub| (*sub).to_string()))
        .collect()
}

pub fn tag_name(tag: &str, zh: bool) -> &'static str {
    topic(tag).map_or("", |topic| if zh { topic.name_zh } else { topic.name_en })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_and_english_interests_map_to_tags() {
        let tags = tags_for_text("我做多模态大模型，也关注 program analysis");
        assert!(tags.contains("multimodal"));
        assert!(tags.contains("llm"));
        assert!(tags.contains("se"));
    }

    #[test]
    fn neural_network_does_not_imply_computer_networks() {
        let tags = tags_for_text("图神经网络 graph neural network");
        assert!(tags.contains("datamining"));
        assert!(!tags.contains("network"));
    }

    #[test]
    fn short_aliases_match_whole_words_only() {
        assert!(tags_for_text("CV and NLP").contains("cv"));
        assert!(!tags_for_text("research on search").contains("graphics"));
        assert!(tags_for_text("AR/VR headsets").contains("graphics"));
    }

    #[test]
    fn venues_get_curated_and_name_based_tags() {
        let cvpr = venue_tags(
            "CVPR",
            "IEEE/CVF Conference on Computer Vision and Pattern Recognition",
        );
        assert!(cvpr.contains("cv"));
        let unknown = venue_tags("XYZ", "International Symposium on Software Testing");
        assert!(unknown.contains("se"));
    }

    #[test]
    fn tags_imply_categories() {
        let tags = tags_for_text("security of LLM agents");
        let subs = subs_for_tags(&tags);
        assert!(subs.contains("SC"));
        assert!(subs.contains("AI"));
    }
}

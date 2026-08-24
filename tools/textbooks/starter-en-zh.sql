PRAGMA journal_mode=DELETE;
CREATE TABLE simple_translation (written_rep TEXT NOT NULL, trans_list TEXT NOT NULL, max_score REAL, rel_importance REAL);
CREATE TABLE translation (lexentry TEXT, written_rep TEXT NOT NULL, trans_list TEXT NOT NULL, score REAL, is_good INTEGER, importance REAL);
INSERT INTO simple_translation(written_rep, trans_list) VALUES
('ability','能力'),('accept','接受'),('achieve','实现'),('action','行动'),('active','积极的'),
('advice','建议'),('agree','同意'),('allow','允许'),('answer','回答'),('appear','出现'),
('apply','申请；应用'),('arrive','到达'),('attention','注意'),('available','可用的'),('avoid','避免'),
('believe','相信'),('benefit','益处'),('build','建造'),('change','改变'),('choose','选择'),
('clear','清楚的'),('common','常见的'),('compare','比较'),('complete','完成'),('consider','考虑'),
('continue','继续'),('create','创建'),('decide','决定'),('develop','发展'),('different','不同的'),
('difficult','困难的'),('discover','发现'),('early','早的'),('easy','容易的'),('effect','影响'),
('enough','足够的'),('example','例子'),('experience','经验'),('explain','解释'),('follow','跟随'),
('future','未来'),('general','一般的'),('happen','发生'),('important','重要的'),('improve','改进'),
('include','包括'),('increase','增加'),('information','信息'),('interest','兴趣'),('language','语言'),
('learn','学习'),('meaning','含义'),('necessary','必要的'),('notice','注意到'),('option','选项'),
('practice','练习'),('prepare','准备'),('problem','问题'),('provide','提供'),('reason','原因'),
('receive','收到'),('remember','记住'),('result','结果'),('select','选择'),('service','服务'),
('similar','相似的'),('simple','简单的'),('source','来源'),('study','学习'),('support','支持'),
('system','系统'),('textbook','教材'),('translate','翻译'),('understand','理解'),('useful','有用的'),
('value','价值'),('vocabulary','词汇'),('window','窗口'),('word','单词'),('work','工作');
INSERT INTO translation(lexentry,written_rep,trans_list,score,is_good,importance)
SELECT written_rep || '__verb__1', written_rep, trans_list, 1, 1, 1 FROM simple_translation WHERE written_rep IN ('accept','achieve','agree','allow','apply','arrive','avoid','believe','build','change','choose','compare','complete','consider','continue','create','decide','develop','discover','explain','follow','happen','improve','include','increase','learn','notice','practice','prepare','provide','receive','remember','select','study','support','translate','understand','work');
INSERT INTO translation(lexentry,written_rep,trans_list,score,is_good,importance)
SELECT written_rep || '__noun__1', written_rep, trans_list, 1, 1, 1 FROM simple_translation WHERE written_rep IN ('ability','action','advice','answer','attention','benefit','effect','example','experience','future','information','interest','language','meaning','option','problem','reason','result','service','source','system','textbook','value','vocabulary','window','word');
INSERT INTO translation(lexentry,written_rep,trans_list,score,is_good,importance)
SELECT written_rep || '__adjective__1', written_rep, trans_list, 1, 1, 1 FROM simple_translation WHERE written_rep IN ('active','available','clear','common','different','difficult','early','easy','enough','general','important','necessary','similar','simple','useful');
VACUUM;

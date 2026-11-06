# 编码矩阵（强版）：多字节样本 + 更多码点
set bytes [binary format H* 41a1c3e9ff0d0a2129]
set chars "A\u00e9\u4e2d\u3042\u20ac\u00ff\u0100\ufffd"
foreach enc [encoding names] {
    catch {encoding convertfrom $enc $bytes} dec
    catch {encoding convertto $enc $chars} enc_out
    puts "$enc | [binary encode hex $dec] | [binary encode hex $enc_out]"
}

/**
 * Vendored HLSL keyword tables from
 * https://github.com/yyqrc/vscode-unitylabshader (MIT)
 * based on https://github.com/stef-levesque/vscode-shader (MIT)
 * and Microsoft HLSL documentation.
 */

export interface IParam {
    label: string;
    documentation?: string;
}

export class IEntry {
    description?: string;
    parameters?: IParam[];
}

 //TODO: support multiple entry per name
export interface IEntries { [name: string]: IEntry; }

// From https://docs.microsoft.com/en-ca/windows/win32/direct3dhlsl/dx-graphics-hlsl-intrinsic-functions

export var intrinsicfunctions: IEntries = {
    abort: {
        description: 'Submits an error message to the information queue and terminates the current draw or dispatch call being executed.\n\n向信息队列提交错误消息并终止当前正在执行的绘制或调度调用。',
    },
    abs: {
        description: 'Returns the absolute value of the specified value.\n\n返回指定值的绝对值。',
        parameters: [{ label: 'value', documentation: 'The specified value | 指定的值' }],
    },
    acos: {
        description: 'Returns the arccosine of the specified value.\n\n返回指定值的反余弦值。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified value. Each component should be a floating-point value within the range of -1 to 1. | 指定的值，每个分量应为-1到1范围内的浮点值。'
            }
        ],
    },
    all: {
        description: 'Determines if all components of the specified value are non-zero.\n\n判断指定值的所有分量是否都非零。',
        parameters: [{ label: 'value', documentation: 'The specified value | 指定的值' }],
    },
    AllMemoryBarrier: {
        description: 'Blocks execution of all threads in a group until all memory accesses have been completed.\n\n阻塞组中所有线程的执行，直到所有内存访问完成。',
    },
    AllMemoryBarrierWithGroupSync: {
        description: 'Blocks execution of all threads in a group until all memory accesses have been completed and all threads in the group have reached this call.\n\n阻塞组中所有线程的执行，直到所有内存访问完成且组中所有线程都到达此调用。',
    },
    any: {
        description: 'Determines if any components of the specified value are non-zero.\n\n判断指定值是否有任意分量非零。',
        parameters: [{ label: 'value', documentation: 'The specified value | 指定的值' }],
    },
    asdouble: {
        description: 'Reinterprets a cast value (two 32-bit values) into a double.\n\n将两个32位值重新解释为双精度浮点数。',
        parameters: [
            {
                label: 'lowbits',
                documentation: 'The low 32-bit pattern of the input value. | 输入值的低32位。'
            },
            {
                label: 'highbits',
                documentation: 'The high 32-bit pattern of the input value. | 输入值的高32位。'
            }
        ],
    },
    asfloat: {
        description: 'Interprets the bit pattern of the input value as a floating-point number.\n\n将输入值的位模式解释为浮点数。',
        parameters: [{ label: 'value', documentation: 'The input value. | 输入值' }],
    },
    asin: {
        description: 'Returns the arcsine of the specified value.\n\n返回指定值的反正弦值。',
        parameters: [{ label: 'value', documentation: 'The specified value | 指定的值' }],
    },
    asint: {
        description: 'Interprets the bit pattern of the input value as an integer.\n\n将输入值的位模式解释为整数。',
        parameters: [{ label: 'value', documentation: 'The input value. | 输入值' }],
    },
    asuint: {
        description: 'Reinterprets the bit pattern of a 64-bit value as two unsigned 32-bit integers.\n\n将64位值的位模式重新解释为两个无符号32位整数。',
        parameters: [
            { label: 'value', documentation: 'The input value. | 输入值' },
            {
                label: 'lowbits',
                documentation: 'The low 32-bit pattern of the input value. | 输入值的低32位。'
            },
            {
                label: 'highbits',
                documentation: 'The high 32-bit pattern of the input value. | 输入值的高32位。'
            }
        ],
    },
    atan: {
        description: 'Returns the arctangent of the specified value.\n\n返回指定值的反正切值。',
        parameters: [{ label: 'value', documentation: 'The specified value | 指定的值' }],
    },
    atan2: {
        description: 'Returns the arctangent of two values (x,y).\n\n返回两个值(x,y)的反正切值。',
        parameters: [
            { label: 'y', documentation: 'The y value. | y值' },
            { label: 'x', documentation: 'The x value. | x值' }
        ],
    },
    ceil: {
        description: 'Returns the smallest integer value that is greater than or equal to the specified value.\n\n返回大于或等于指定值的最小整数（向上取整）。',
        parameters: [{ label: 'value', documentation: 'The specified value | 指定的值' }],
    },
    CheckAccessFullyMapped: {
        description: 'Determines whether all values from a Sample, Gather, or Load operation accessed mapped tiles in a tiled resource.\n\n确定 Sample、Gather 或 Load 操作的所有值是否都访问了平铺资源中的映射图块。',
        parameters: [
            {
                label: 'status',
                documentation: "The status value that is returned from a Sample, Gather, or Load operation. | 从 Sample、Gather 或 Load 操作返回的状态值。"
            }
        ],
    },
    clamp: {
        description: 'Clamps the specified value to the specified minimum and maximum range.\n\n将指定值限制在指定的最小和最大范围内。',
        parameters: [
            { label: 'value', documentation: 'A value to clamp. | 要限制的值' },
            { label: 'min', documentation: 'The specified minimum range. | 指定的最小范围' },
            { label: 'max', documentation: 'The specified maximum range. | 指定的最大范围' }
        ],
    },
    clip: {
        description: 'Discards the current pixel if the specified value is less than zero.\n\n如果指定值小于零，则丢弃当前像素。',
        parameters: [{ label: 'value', documentation: 'The specified value | 指定的值' }],
    },
    cos: {
        description: 'Returns the cosine of the specified value.\n\n返回指定值的余弦值。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified value, in radians. | 指定的值，以弧度为单位'
            }
        ],
    },
    cosh: {
        description: 'Returns the hyperbolic cosine of the specified value.\n\n返回指定值的双曲余弦值。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified value, in radians. | 指定的值，以弧度为单位'
            }
        ],
    },
    countbits: {
        description: 'Counts the number of bits (per component) in the input integer.\n\n计算输入整数中（每个分量）的位数。',
        parameters: [{ label: 'value', documentation: 'The input value. | 输入值' }],
    },
    cross: {
        description: 'Returns the cross product of two floating-point, 3D vectors.\n\n返回两个浮点型三维向量的叉积。',
        parameters: [
            {
                label: 'x',
                documentation: 'The first floating-point, 3D vector. | 第一个浮点型三维向量'
            },
            {
                label: 'y',
                documentation: 'The second floating-point, 3D vector. | 第二个浮点型三维向量'
            }
        ],
    },
    D3DCOLORtoUBYTE4: {
        description: 'Converts a floating-point, 4D vector set by a D3DCOLOR to a UBYTE4.\n\n将 D3DCOLOR 设置的浮点型四维向量转换为 UBYTE4。',
        parameters: [
            {
                label: 'value',
                documentation: 'The floating-point vector4 to convert. | 要转换的浮点型四维向量'
            }
        ],
    },
    ddx: {
        description: 'Returns the partial derivative of the specified value with respect to the screen-space x-coordinate.\n\n返回指定值关于屏幕空间 x 坐标的偏导数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    ddx_coarse: {
        description: 'Computes a low precision partial derivative with respect to the screen-space x-coordinate.\n\n计算关于屏幕空间 x 坐标的低精度偏导数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    ddx_fine: {
        description: 'Computes a high precision partial derivative with respect to the screen-space x-coordinate.\n\n计算关于屏幕空间 x 坐标的高精度偏导数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    ddy: {
        description: 'Returns the partial derivative of the specified value with respect to the screen-space y-coordinate.\n\n返回指定值关于屏幕空间 y 坐标的偏导数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    ddy_coarse: {
        description: 'Computes a low precision partial derivative with respect to the screen-space y-coordinate.\n\n计算关于屏幕空间 y 坐标的低精度偏导数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    ddy_fine: {
        description: 'Computes a high precision partial derivative with respect to the screen-space y-coordinate.\n\n计算关于屏幕空间 y 坐标的高精度偏导数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    degrees: {
        description: 'Converts the specified value from radians to degrees.\n\n将指定值从弧度转换为度数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    determinant: {
        description: 'Returns the determinant of the specified floating-point, square matrix.\n\n返回指定浮点型方阵的行列式。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    DeviceMemoryBarrier: {
        description: 'Blocks execution of all threads in a group until all device memory accesses have been completed.\n\n阻塞组中所有线程的执行，直到所有设备内存访问完成。',
    },
    DeviceMemoryBarrierWithGroupSync: {
        description: 'Blocks execution of all threads in a group until all device memory accesses have been completed and all threads in the group have reached this call.\n\n阻塞组中所有线程的执行，直到所有设备内存访问完成且组中所有线程都到达此调用。',
    },
    distance: {
        description: 'Returns a distance scalar between two vectors.\n\n返回两个向量之间的距离标量。',
        parameters: [
            {
                label: 'x',
                documentation: 'The first floating-point vector to compare. | 第一个要比较的浮点型向量'
            },
            {
                label: 'y',
                documentation: 'The second floating-point vector to compare. | 第二个要比较的浮点型向量'
            }
        ],
    },
    dot: {
        description: 'Returns the dot product of two vectors.\n\n返回两个向量的点积。',
        parameters: [
            { label: 'x', documentation: 'The first vector. | 第一个向量' },
            { label: 'y', documentation: 'The second vector. | 第二个向量' }
        ],
    },
    dst: {
        description: 'Calculates a distance vector.\n\n计算距离向量。',
        parameters: [
            { label: 'x', documentation: 'The first vector. | 第一个向量' },
            { label: 'y', documentation: 'The second vector. | 第二个向量' }
        ],
    },
    errorf: {
        description: 'Submits an error message to the information queue.\n\n向信息队列提交错误消息。',
        parameters: [
            { label: 'format', documentation: 'The format string. | 格式化字符串' },
            { label: 'argument ...', documentation: 'Optional arguments. | 可选参数' }
        ],
    },
    EvaluateAttributeAtCentroid: {
        description: 'Evaluates at the pixel centroid.\n\n在像素质心处计算。',
        parameters: [{ label: 'value', documentation: 'The input value. | 输入值' }],
    },
    EvaluateAttributeAtSample: {
        description: 'Evaluates at the indexed sample location.\n\n在指定的采样位置计算。',
        parameters: [
            { label: 'value', documentation: 'The input value. | 输入值' },
            { label: 'sampleIndex', documentation: 'The sample location. | 采样位置' }
        ],
    },
    EvaluateAttributeSnapped: {
        description: 'Evaluates at the pixel centroid with an offset.\n\n在像素质心处带偏移计算。',
        parameters: [
            { label: 'value', documentation: 'The input value. | 输入值' },
            {
                label: 'offset',
                documentation: 'A 2D offset from the pixel center using a 16x16 grid. | 使用16x16网格从像素中心的二维偏移'
            }
        ],
    },
    exp: {
        description: 'Returns the base-e exponential, or e^x, of the specified value.\n\n返回指定值的以 e 为底的指数（e^x）。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    exp2: {
        description: 'Returns the base 2 exponential, or 2^x, of the specified value.\n\n返回指定值的以 2 为底的指数（2^x）。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    f16tof32: {
        description: 'Converts the float16 stored in the low-half of the uint to a float.\n\n将存储在 uint 低半部分的 float16 转换为 float。',
        parameters: [{ label: 'value', documentation: 'The input value. | 输入值' }],
    },
    f32tof16: {
        description: 'Converts an input into a float16 type.\n\n将输入转换为 float16 类型。',
        parameters: [{ label: 'value', documentation: 'The input value. | 输入值' }],
    },
    faceforward: {
        description: 'Flips the surface-normal (if needed) to face in a direction opposite to i; returns the result in n.\n\n翻转表面法线（如果需要）使其面向与 i 相反的方向，并返回 n 中的结果。',
        parameters: [
            {
                label: 'n',
                documentation: 'The resulting floating-point surface-normal vector. | 结果浮点型表面法线向量'
            },
            {
                label: 'i',
                documentation: 'A floating-point, incident vector that points from the view position to the shading position. | 从观察位置指向着色位置的浮点型入射向量'
            },
            {
                label: 'ng',
                documentation: 'A floating-point surface-normal vector. | 浮点型表面法线向量'
            }
        ],
    },
    firstbithigh: {
        description: 'Gets the location of the first set bit starting from the highest order bit and working downward, per component.\n\n从最高位开始向下查找，获取第一个置位为1的位置（每个分量）。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    firstbitlow: {
        description: 'Returns the location of the first set bit starting from the lowest order bit and working upward, per component.\n\n从最低位开始向上查找，返回第一个置位为1的位置（每个分量）。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    floor: {
        description: 'Returns the largest integer that is less than or equal to the specified value.\n\n返回小于或等于指定值的最大整数（向下取整）。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    fma: {
        description: 'Returns the double-precision fused multiply-addition of a * b + c.\n\n返回 a * b + c 的双精度融合乘加结果。',
        parameters: [
            { label: 'a', documentation: 'The first value in the fused multiply-addition. | 融合乘加的第一个值' },
            { label: 'b', documentation: 'The second value in the fused multiply-addition. | 融合乘加的第二个值' },
            { label: 'c', documentation: 'The third value in the fused multiply-addition. | 融合乘加的第三个值' }
        ],
    },
    fmod: {
        description: 'Returns the floating-point remainder of x/y.\n\n返回 x/y 的浮点余数。',
        parameters: [
            { label: 'x', documentation: 'The floating-point dividend. | 浮点被除数' },
            { label: 'y', documentation: 'The floating-point divisor. | 浮点除数' }
        ],
    },
    frac: {
        description: 'Returns the fractional (or decimal) part of x; which is greater than or equal to 0 and less than 1.\n\n返回 x 的小数部分，结果大于等于 0 且小于 1。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    frexp: {
        description: 'Returns the mantissa and exponent of the specified floating-point value.\n\n返回指定浮点值的尾数和指数。',
        parameters: [
            { label: 'x', documentation: 'The specified floating-point value. | 指定的浮点值' },
            { label: 'exp', documentation: 'The returned exponent of the x parameter. | 返回的 x 参数的指数' }
        ],
    },
    fwidth: {
        description: 'Returns the absolute value of the partial derivatives of the specified value.\n\n返回指定值偏导数的绝对值。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    GetRenderTargetSampleCount: {
        description: 'Gets the number of samples for a render target.\n\n获取渲染目标的采样数。',
    },
    GetRenderTargetSamplePosition: {
        description: 'Gets the sampling position (x,y) for a given sample index.\n\n获取给定采样索引的采样位置 (x,y)。',
        parameters: [
            { label: 'index', documentation: 'A zero-based sample index. | 以零为基础的采样索引' }
        ],
    },
    GroupMemoryBarrier: {
        description: 'Blocks execution of all threads in a group until all group shared accesses have been completed.\n\n阻塞组中所有线程的执行，直到所有组共享访问完成。',
    },
    GroupMemoryBarrierWithGroupSync: {
        description: 'Blocks execution of all threads in a group until all group shared accesses have been completed and all threads in the group have reached this call.\n\n阻塞组中所有线程的执行，直到所有组共享访问完成且组中所有线程都到达此调用。',
    },
    InterlockedAdd: {
        description: 'Performs a guaranteed atomic add of value to the dest resource variable.\n\n对目标资源变量执行保证原子性的加法操作。',
        parameters: [
            { label: 'dest', documentation: 'The destination address. | 目标地址' },
            { label: 'value', documentation: 'The input value. | 输入值' },
            {
                label: 'originalValue',
                documentation: 'Optional. The original input value. | 可选，原始输入值'
            }
        ],
    },
    InterlockedAnd: {
        description: 'Performs a guaranteed atomic and.\n\n执行保证原子性的按位与操作。',
        parameters: [
            { label: 'dest', documentation: 'The destination address. | 目标地址' },
            { label: 'value', documentation: 'The input value. | 输入值' },
            {
                label: 'originalValue',
                documentation: 'Optional. The original input value. | 可选，原始输入值'
            }
        ],
    },
    InterlockedCompareExchange: {
        description: "Atomically compares the destination with the comparison value. If they are identical, the destination is overwritten with the input value.\n\n原子比较目标值与比较值，如果相同则用输入值覆盖目标值。",
        parameters: [
            { label: 'dest', documentation: 'The destination address. | 目标地址' },
            {
                label: 'compareValue',
                documentation: 'The comparison value. | 比较值'
            },
            { label: 'value', documentation: 'The input value. | 输入值' },
            {
                label: 'originalValue',
                documentation: 'The original value. | 原始值'
            }
        ],
    },
    InterlockedCompareStore: {
        description: 'Atomically compares the destination to the comparison value. If they are identical, the destination is overwritten with the input value.\n\n原子比较目标值与比较值，如果相同则用输入值覆盖目标值。',
        parameters: [
            { label: 'dest', documentation: 'The destination address. | 目标地址' },
            {
                label: 'compareValue',
                documentation: 'The comparison value. | 比较值'
            },
            { label: 'value', documentation: 'The input value. | 输入值' }
        ],
    },
    InterlockedExchange: {
        description: 'Assigns value to dest and returns the original value.\n\n将值赋给目标并返回原始值。',
        parameters: [
            { label: 'dest', documentation: 'The destination address. | 目标地址' },
            { label: 'value', documentation: 'The input value. | 输入值' },
            {
                label: 'originalValue',
                documentation: 'The original input value. | 原始输入值'
            }
        ],
    },
    InterlockedMax: {
        description: 'Performs a guaranteed atomic max.\n\n执行保证原子性的最大值操作。',
        parameters: [
            { label: 'dest', documentation: 'The destination address. | 目标地址' },
            { label: 'value', documentation: 'The input value. | 输入值' },
            {
                label: 'originalValue',
                documentation: 'Optional. The original input value. | 可选，原始输入值'
            }
        ],
    },
    InterlockedMin: {
        description: 'Performs a guaranteed atomic min.\n\n执行保证原子性的最小值操作。',
        parameters: [
            { label: 'dest', documentation: 'The destination address. | 目标地址' },
            { label: 'value', documentation: 'The input value. | 输入值' },
            {
                label: 'originalValue',
                documentation: 'Optional. The original input value. | 可选，原始输入值'
            }
        ],
    },
    InterlockedOr: {
        description: 'Performs a guaranteed atomic or.\n\n执行保证原子性的按位或操作。',
        parameters: [
            { label: 'dest', documentation: 'The destination address. | 目标地址' },
            { label: 'value', documentation: 'The input value. | 输入值' },
            {
                label: 'originalValue',
                documentation: 'Optional. The original input value. | 可选，原始输入值'
            }
        ],
    },
    InterlockedXor: {
        description: 'Performs a guaranteed atomic xor.\n\n执行保证原子性的按位异或操作。',
        parameters: [
            { label: 'dest', documentation: 'The destination address. | 目标地址' },
            { label: 'value', documentation: 'The input value. | 输入值' },
            {
                label: 'originalValue',
                documentation: 'Optional. The original input value. | 可选，原始输入值'
            }
        ],
    },
    isfinite: {
        description: 'Determines if the specified floating-point value is finite.\n\n判断指定的浮点值是否为有限值。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    isinf: {
        description: 'Determines if the specified value is infinite.\n\n判断指定值是否为无穷大。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    isnan: {
        description: 'Determines if the specified value is NAN or QNAN.\n\n判断指定值是否为 NaN（非数字）。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    ldexp: {
        description: 'Returns the result of multiplying the specified value by two, raised to the power of the specified exponent.\n\n返回指定值乘以 2 的指定次幂的结果。',
        parameters: [
            { label: 'value', documentation: 'The specified value. | 指定的值' },
            { label: 'exp', documentation: 'The specified exponent. | 指定的指数' }
        ],
    },
    length: {
        description: 'Returns the length of the specified floating-point vector.\n\n返回指定浮点向量的长度（模）。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified floating-point vector. | 指定的浮点向量'
            }
        ],
    },
    lerp: {
        description: 'Performs a linear interpolation.\n\n执行线性插值。',
        parameters: [
            {
                label: 'x',
                documentation: 'The first floating-point value. | 第一个浮点值'
            },
            {
                label: 'y',
                documentation: 'The second floating-point value. | 第二个浮点值'
            },
            {
                label: 's',
                documentation: 'A value that linearly interpolates between the x parameter and the y parameter. | 在 x 和 y 之间进行线性插值的值'
            }
        ],
    },
    lit: {
        description: 'Returns a lighting coefficient vector.\n\n返回光照系数向量。',
        parameters: [
            {
                label: 'nDotL',
                documentation: 'The dot product of the normalized surface normal and the light vector. | 归一化表面法线与光线向量的点积'
            },
            {
                label: 'nDotH',
                documentation: 'The dot product of the half-angle vector and the surface normal. | 半角向量与表面法线的点积'
            },
            { label: 'm', documentation: 'A specular exponent. | 高光指数' }
        ],
    },
    log: {
        description: 'Returns the base-e logarithm of the specified value.\n\n返回指定值的自然对数（以 e 为底）。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    log10: {
        description: 'Returns the base-10 logarithm of the specified value.\n\n返回指定值的以 10 为底的对数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    log2: {
        description: 'Returns the base-2 logarithm of the specified value.\n\n返回指定值的以 2 为底的对数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    mad: {
        description: 'Performs an arithmetic multiply/add operation on three values. Returns the result of x * y + a.\n\n对三个值执行乘加运算，返回 x * y + a 的结果。',
        parameters: [
            { label: 'x', documentation: 'The first multiplication value. | 第一个乘数' },
            { label: 'y', documentation: 'The second multiplication value. | 第二个乘数' },
            { label: 'a', documentation: 'The addition value. | 加数' }
        ],
    },
    max: {
        description: 'Selects the greater of x and y.\n\n返回 x 和 y 中较大的值。',
        parameters: [
            { label: 'x', documentation: 'The x input value. | x 输入值' },
            { label: 'y', documentation: 'The y input value. | y 输入值' }
        ],
    },
    min: {
        description: 'Selects the lesser of x and y.\n\n返回 x 和 y 中较小的值。',
        parameters: [
            { label: 'x', documentation: 'The x input value. | x 输入值' },
            { label: 'y', documentation: 'The y input value. | y 输入值' }
        ],
    },
    modf: {
        description: 'Splits the value x into fractional and integer parts, each of which has the same sign as x.\n\n将值 x 分解为小数部分和整数部分，两者的符号与 x 相同。',
        parameters: [
            { label: 'x', documentation: 'The x input value. | x 输入值' },
            { label: 'ip', documentation: 'The integer portion of x. | x 的整数部分' }
        ],
    },
    msad4: {
        description: 'Compares a 4-byte reference value and an 8-byte source value and accumulates a vector of 4 sums.\n\n比较4字节参考值和8字节源值，并累加4个和的向量。',
        parameters: [
            {
                label: 'reference',
                documentation: 'The reference array of 4 bytes in one uint value. | 一个 uint 值中的4字节参考数组'
            },
            {
                label: 'source',
                documentation: 'The source array of 8 bytes in two uint2 values. | 两个 uint2 值中的8字节源数组'
            },
            {
                label: 'accum',
                documentation: 'A vector of 4 values to accumulate. | 要累加的4个值的向量'
            }
        ],
    },
    mul: {
        description: 'Multiplies x and y using matrix math. The inner dimension x-columns and y-rows must be equal.\n\n使用矩阵运算将 x 和 y 相乘，x 的列数和 y 的行数必须相等。',
        parameters: [
            {
                label: 'x',
                documentation: 'The x input value. If x is a vector, it treated as a row vector. | x 输入值。如果 x 是向量，则视为行向量'
            },
            {
                label: 'y',
                documentation: 'The y input value. If y is a vector, it treated as a column vector. | y 输入值。如果 y 是向量，则视为列向量'
            }
        ],
    },
    noise: {
        description: 'Generates a random value using the Perlin-noise algorithm.\n\n使用 Perlin 噪声算法生成随机值。',
        parameters: [
            {
                label: 'value',
                documentation: 'A floating-point vector from which to generate Perlin noise. | 用于生成 Perlin 噪声的浮点向量'
            }
        ],
    },
    normalize: {
        description: 'Normalizes the specified floating-point vector according to x / length(x).\n\n按照 x / length(x) 归一化指定的浮点向量。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified floating-point vector. | 指定的浮点向量'
            }
        ],
    },
    pow: {
        description: 'Returns the specified value raised to the specified power.\n\n返回指定值的指定次幂。',
        parameters: [
            { label: 'x', documentation: 'The specified value. | 指定的值（底数）' },
            { label: 'y', documentation: 'The specified power. | 指定的幂（指数）' }
        ],
    },
    printf: {
        description: 'Submits a custom shader message to the information queue.\n\n向信息队列提交自定义着色器消息。',
        parameters: [
            { label: 'format', documentation: 'The format string. | 格式化字符串' },
            { label: 'argument ...', documentation: 'Optional arguments. | 可选参数' }
        ],
    },
    Process2DQuadTessFactorsAvg: {
        description: 'Generates the corrected tessellation factors for a quad patch (using average).\n\n生成四边形面片的校正细分因子（使用平均值）。',
        parameters: [
            { label: 'RawEdgeFactors', documentation: 'The edge tessellation factors. | 边缘细分因子' },
            { label: 'InsideScale', documentation: 'The scale factor (0.0 to 1.0). | 缩放因子（0.0 到 1.0）' },
            { label: 'RoundedEdgeTessFactors', documentation: 'The rounded edge factors. | 四舍五入的边缘因子' },
            { label: 'RoundedInsideTessFactors', documentation: 'The rounded inside factors. | 四舍五入的内部因子' },
            { label: 'UnroundedInsideTessFactors', documentation: 'The unrounded inside factors. | 未四舍五入的内部因子' }
        ],
    },
    Process2DQuadTessFactorsMax: {
        description: 'Generates the corrected tessellation factors for a quad patch (using maximum).\n\n生成四边形面片的校正细分因子（使用最大值）。',
        parameters: [
            { label: 'RawEdgeFactors', documentation: 'The edge tessellation factors. | 边缘细分因子' },
            { label: 'InsideScale', documentation: 'The scale factor (0.0 to 1.0). | 缩放因子（0.0 到 1.0）' },
            { label: 'RoundedEdgeTessFactors', documentation: 'The rounded edge factors. | 四舍五入的边缘因子' },
            { label: 'RoundedInsideTessFactors', documentation: 'The rounded inside factors. | 四舍五入的内部因子' },
            { label: 'UnroundedInsideTessFactors', documentation: 'The unrounded inside factors. | 未四舍五入的内部因子' }
        ],
    },
    Process2DQuadTessFactorsMin: {
        description: 'Generates the corrected tessellation factors for a quad patch (using minimum).\n\n生成四边形面片的校正细分因子（使用最小值）。',
        parameters: [
            { label: 'RawEdgeFactors', documentation: 'The edge tessellation factors. | 边缘细分因子' },
            { label: 'InsideScale', documentation: 'The scale factor (0.0 to 1.0). | 缩放因子（0.0 到 1.0）' },
            { label: 'RoundedEdgeTessFactors', documentation: 'The rounded edge factors. | 四舍五入的边缘因子' },
            { label: 'RoundedInsideTessFactors', documentation: 'The rounded inside factors. | 四舍五入的内部因子' },
            { label: 'UnroundedInsideTessFactors', documentation: 'The unrounded inside factors. | 未四舍五入的内部因子' }
        ],
    },
    ProcessIsolineTessFactors: {
        description: 'Generates the rounded tessellation factors for an isoline.\n\n生成等值线的四舍五入细分因子。',
        parameters: [
            { label: 'RawDetailFactor', documentation: 'The desired detail factor. | 期望的细节因子' },
            { label: 'RawDensityFactor', documentation: 'The desired density factor. | 期望的密度因子' },
            { label: 'RoundedDetailFactor', documentation: 'The rounded detail factor. | 四舍五入的细节因子' },
            { label: 'RoundedDensityFactor', documentation: 'The rounded density factor. | 四舍五入的密度因子' }
        ],
    },
    ProcessQuadTessFactorsAvg: {
        description: 'Generates the corrected tessellation factors for a quad patch (using average).\n\n生成四边形面片的校正细分因子（使用平均值）。',
        parameters: [
            { label: 'RawEdgeFactors', documentation: 'The edge tessellation factors. | 边缘细分因子' },
            { label: 'InsideScale', documentation: 'The scale factor (0.0 to 1.0). | 缩放因子（0.0 到 1.0）' },
            { label: 'RoundedEdgeTessFactors', documentation: 'The rounded edge factors. | 四舍五入的边缘因子' },
            { label: 'RoundedInsideTessFactors', documentation: 'The rounded inside factors. | 四舍五入的内部因子' },
            { label: 'UnroundedInsideTessFactors', documentation: 'The unrounded inside factors. | 未四舍五入的内部因子' }
        ],
    },
    ProcessQuadTessFactorsMax: {
        description: 'Generates the corrected tessellation factors for a quad patch (using maximum).\n\n生成四边形面片的校正细分因子（使用最大值）。',
        parameters: [
            { label: 'RawEdgeFactors', documentation: 'The edge tessellation factors. | 边缘细分因子' },
            { label: 'InsideScale', documentation: 'The scale factor (0.0 to 1.0). | 缩放因子（0.0 到 1.0）' },
            { label: 'RoundedEdgeTessFactors', documentation: 'The rounded edge factors. | 四舍五入的边缘因子' },
            { label: 'RoundedInsideTessFactors', documentation: 'The rounded inside factors. | 四舍五入的内部因子' },
            { label: 'UnroundedInsideTessFactors', documentation: 'The unrounded inside factors. | 未四舍五入的内部因子' }
        ],
    },
    ProcessQuadTessFactorsMin: {
        description: 'Generates the corrected tessellation factors for a quad patch (using minimum).\n\n生成四边形面片的校正细分因子（使用最小值）。',
        parameters: [
            { label: 'RawEdgeFactors', documentation: 'The edge tessellation factors. | 边缘细分因子' },
            { label: 'InsideScale', documentation: 'The scale factor (0.0 to 1.0). | 缩放因子（0.0 到 1.0）' },
            { label: 'RoundedEdgeTessFactors', documentation: 'The rounded edge factors. | 四舍五入的边缘因子' },
            { label: 'RoundedInsideTessFactors', documentation: 'The rounded inside factors. | 四舍五入的内部因子' },
            { label: 'UnroundedInsideTessFactors', documentation: 'The unrounded inside factors. | 未四舍五入的内部因子' }
        ],
    },
    ProcessTriTessFactorsAvg: {
        description: 'Generates the corrected tessellation factors for a triangle patch (using average).\n\n生成三角形面片的校正细分因子（使用平均值）。',
        parameters: [
            { label: 'RawEdgeFactors', documentation: 'The edge tessellation factors. | 边缘细分因子' },
            { label: 'InsideScale', documentation: 'The scale factor (0.0 to 1.0). | 缩放因子（0.0 到 1.0）' },
            { label: 'RoundedEdgeTessFactors', documentation: 'The rounded edge factors. | 四舍五入的边缘因子' },
            { label: 'RoundedInsideTessFactors', documentation: 'The rounded inside factors. | 四舍五入的内部因子' },
            { label: 'UnroundedInsideTessFactors', documentation: 'The unrounded inside factors. | 未四舍五入的内部因子' }
        ],
    },
    ProcessTriTessFactorsMax: {
        description: 'Generates the corrected tessellation factors for a triangle patch (using maximum).\n\n生成三角形面片的校正细分因子（使用最大值）。',
        parameters: [
            { label: 'RawEdgeFactors', documentation: 'The edge tessellation factors. | 边缘细分因子' },
            { label: 'InsideScale', documentation: 'The scale factor (0.0 to 1.0). | 缩放因子（0.0 到 1.0）' },
            { label: 'RoundedEdgeTessFactors', documentation: 'The rounded edge factors. | 四舍五入的边缘因子' },
            { label: 'RoundedInsideTessFactors', documentation: 'The rounded inside factors. | 四舍五入的内部因子' },
            { label: 'UnroundedInsideTessFactors', documentation: 'The unrounded inside factors. | 未四舍五入的内部因子' }
        ],
    },
    ProcessTriTessFactorsMin: {
        description: 'Generates the corrected tessellation factors for a triangle patch (using minimum).\n\n生成三角形面片的校正细分因子（使用最小值）。',
        parameters: [
            { label: 'RawEdgeFactors', documentation: 'The edge tessellation factors. | 边缘细分因子' },
            { label: 'InsideScale', documentation: 'The scale factor (0.0 to 1.0). | 缩放因子（0.0 到 1.0）' },
            { label: 'RoundedEdgeTessFactors', documentation: 'The rounded edge factors. | 四舍五入的边缘因子' },
            { label: 'RoundedInsideTessFactors', documentation: 'The rounded inside factors. | 四舍五入的内部因子' },
            { label: 'UnroundedInsideTessFactors', documentation: 'The unrounded inside factors. | 未四舍五入的内部因子' }
        ],
    },
    radians: {
        description: 'Converts the specified value from degrees to radians.\n\n将指定值从度数转换为弧度。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    rcp: {
        description: 'Calculates a fast, approximate, per-component reciprocal.\n\n计算快速的、近似的、每分量的倒数。',
        parameters: [{ label: 'value', documentation: 'The input value. | 输入值' }],
    },
    reflect: {
        description: 'Returns a reflection vector using an incident ray and a surface normal.\n\n使用入射光线和表面法线返回反射向量。',
        parameters: [
            {
                label: 'i',
                documentation: 'A floating-point, incident vector. | 浮点型入射向量'
            },
            {
                label: 'n',
                documentation: 'A floating-point, normal vector. | 浮点型法线向量'
            }
        ],
    },
    refract: {
        description: 'Returns a refraction vector using an entering ray, a surface normal, and a refraction index.\n\n使用入射光线、表面法线和折射率返回折射向量。',
        parameters: [
            {
                label: 'i',
                documentation: 'A floating-point, ray direction vector. | 浮点型光线方向向量'
            },
            {
                label: 'n',
                documentation: 'A floating-point, surface normal vector. | 浮点型表面法线向量'
            },
            {
                label: 'η',
                documentation: 'A floating-point, refraction index scalar. | 浮点型折射率标量'
            }
        ],
    },
    reversebits: {
        description: 'Reverses the order of the bits, per component.\n\n每个分量反转位的顺序。',
        parameters: [{ label: 'value', documentation: 'The input value. | 输入值' }],
    },
    round: {
        description: 'Rounds the specified value to the nearest integer.\n\n将指定值四舍五入到最近的整数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    rsqrt: {
        description: 'Returns the reciprocal of the square root of the specified value.\n\n返回指定值平方根的倒数。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    saturate: {
        description: 'Clamps the specified value within the range of 0 to 1.\n\n将指定值限制在 0 到 1 的范围内。',
        parameters: [{ label: 'value', documentation: 'The specified value. | 指定的值' }],
    },
    sign: {
        description: 'Returns the sign of x.\n\n返回 x 的符号（负数返回-1，零返回0，正数返回1）。',
        parameters: [{ label: 'value', documentation: 'The input value. | 输入值' }],
    },
    sin: {
        description: 'Returns the sine of the specified value.\n\n返回指定值的正弦值。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified value, in radians. | 指定的值，以弧度为单位'
            }
        ],
    },
    sincos: {
        description: 'Returns the sine and cosine of x.\n\n同时返回 x 的正弦和余弦值。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified value, in radians. | 指定的值，以弧度为单位'
            },
            { label: 's', documentation: 'Returns the sine of x. | 返回 x 的正弦值' },
            { label: 'c', documentation: 'Returns the cosine of x. | 返回 x 的余弦值' }
        ],
    },
    sinh: {
        description: 'Returns the hyperbolic sine of the specified value.\n\n返回指定值的双曲正弦值。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified value, in radians. | 指定的值，以弧度为单位'
            }
        ],
    },
    smoothstep: {
        description: 'Returns a smooth Hermite interpolation between 0 and 1, if x is in the range [min, max].\n\n如果 x 在 [min, max] 范围内，返回 0 和 1 之间的平滑 Hermite 插值。',
        parameters: [
            {
                label: 'min',
                documentation: 'The minimum range of the x parameter. | x 参数的最小范围'
            },
            {
                label: 'max',
                documentation: 'The maximum range of the x parameter. | x 参数的最大范围'
            },
            {
                label: 'x',
                documentation: 'The specified value to be interpolated. | 要插值的指定值'
            }
        ],
    },
    sqrt: {
        description: 'Returns the square root of the specified floating-point value, per component.\n\n返回指定浮点值的平方根（每个分量）。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified floating-point value. | 指定的浮点值'
            }
        ],
    },
    step: {
        description: 'Compares two values, returning 0 or 1 based on which value is greater.\n\n比较两个值，根据哪个值较大返回 0 或 1。',
        parameters: [
            {
                label: 'y',
                documentation: 'The first floating-point value to compare. | 第一个要比较的浮点值'
            },
            {
                label: 'x',
                documentation: 'The second floating-point value to compare. | 第二个要比较的浮点值'
            }
        ],
    },
    tan: {
        description: 'Returns the tangent of the specified value.\n\n返回指定值的正切值。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified value, in radians. | 指定的值，以弧度为单位'
            }
        ],
    },
    tanh: {
        description: 'Returns the hyperbolic tangent of the specified value.\n\n返回指定值的双曲正切值。',
        parameters: [
            {
                label: 'value',
                documentation: 'The specified value, in radians. | 指定的值，以弧度为单位'
            }
        ],
    },
    tex1D: {
        description: 'Samples a 1D texture.\n\n采样一维纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex1Dbias: {
        description: 'Samples a 1D texture after biasing the mip level by t.w.\n\n在通过 t.w 偏移 mip 级别后采样一维纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex1Dgrad: {
        description: 'Samples a 1D texture using a gradient to select the mip level.\n\n使用梯度选择 mip 级别采样一维纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' },
            {
                label: 'ddx',
                documentation: 'Rate of change of the surface geometry in the x direction. | 表面几何在 x 方向的变化率'
            },
            {
                label: 'ddy',
                documentation: 'Rate of change of the surface geometry in the y direction. | 表面几何在 y 方向的变化率'
            }
        ],
    },
    tex1Dlod: {
        description: 'Samples a 1D texture with mipmaps. The mipmap LOD is specified in t.w.\n\n带 mipmap 采样一维纹理，mipmap LOD 在 t.w 中指定。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex1Dproj: {
        description: 'Samples a 1D texture using a projective divide; the texture coordinate is divided by t.w before the lookup takes place.\n\n使用投影除法采样一维纹理，纹理坐标在查找前除以 t.w。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex2D: {
        description: 'Samples a 2D texture.\n\n采样二维纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex2Dbias: {
        description: 'Samples a 2D texture after biasing the mip level by t.w.\n\n在通过 t.w 偏移 mip 级别后采样二维纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex2Dgrad: {
        description: 'Samples a 2D texture using a gradient to select the mip level.\n\n使用梯度选择 mip 级别采样二维纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' },
            {
                label: 'ddx',
                documentation: 'Rate of change of the surface geometry in the x direction. | 表面几何在 x 方向的变化率'
            },
            {
                label: 'ddy',
                documentation: 'Rate of change of the surface geometry in the y direction. | 表面几何在 y 方向的变化率'
            }
        ],
    },
    tex2Dlod: {
        description: 'Samples a 2D texture with mipmaps. The mipmap LOD is specified in t.w.\n\n带 mipmap 采样二维纹理，mipmap LOD 在 t.w 中指定。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex2Dproj: {
        description: 'Samples a 2D texture using a projective divide; the texture coordinate is divided by t.w before the lookup takes place.\n\n使用投影除法采样二维纹理，纹理坐标在查找前除以 t.w。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex3D: {
        description: 'Samples a 3D texture.\n\n采样三维纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex3Dbias: {
        description: 'Samples a 3D texture after biasing the mip level by t.w.\n\n在通过 t.w 偏移 mip 级别后采样三维纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex3Dgrad: {
        description: 'Samples a 3D texture using a gradient to select the mip level.\n\n使用梯度选择 mip 级别采样三维纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' },
            {
                label: 'ddx',
                documentation: 'Rate of change of the surface geometry in the x direction. | 表面几何在 x 方向的变化率'
            },
            {
                label: 'ddy',
                documentation: 'Rate of change of the surface geometry in the y direction. | 表面几何在 y 方向的变化率'
            }
        ],
    },
    tex3Dlod: {
        description: 'Samples a 3D texture with mipmaps. The mipmap LOD is specified in t.w.\n\n带 mipmap 采样三维纹理，mipmap LOD 在 t.w 中指定。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    tex3Dproj: {
        description: 'Samples a 3D texture using a projective divide; the texture coordinate is divided by t.w before the lookup takes place.\n\n使用投影除法采样三维纹理，纹理坐标在查找前除以 t.w。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    texCUBE: {
        description: 'Samples a cube texture.\n\n采样立方体纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    texCUBEbias: {
        description: 'Samples a cube texture after biasing the mip level by t.w.\n\n在通过 t.w 偏移 mip 级别后采样立方体纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    texCUBEgrad: {
        description: 'Samples a cube texture using a gradient to select the mip level.\n\n使用梯度选择 mip 级别采样立方体纹理。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' },
            {
                label: 'ddx',
                documentation: 'Rate of change of the surface geometry in the x direction. | 表面几何在 x 方向的变化率'
            },
            {
                label: 'ddy',
                documentation: 'Rate of change of the surface geometry in the y direction. | 表面几何在 y 方向的变化率'
            }
        ],
    },
    texCUBElod: {
        description: 'Samples a cube texture with mipmaps. The mipmap LOD is specified in t.w.\n\n带 mipmap 采样立方体纹理，mipmap LOD 在 t.w 中指定。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    texCUBEproj: {
        description: 'Samples a cube texture using a projective divide; the texture coordinate is divided by t.w before the lookup takes place.\n\n使用投影除法采样立方体纹理，纹理坐标在查找前除以 t.w。',
        parameters: [
            { label: 's', documentation: 'The sampler state. | 采样器状态' },
            { label: 't', documentation: 'The texture coordinate. | 纹理坐标' }
        ],
    },
    transpose: {
        description: 'Transposes the specified input matrix.\n\n转置指定的输入矩阵。',
        parameters: [{ label: 'value', documentation: 'The specified matrix. | 指定的矩阵' }],
    },
    trunc: {
        description: 'Truncates a floating-point value to the integer component.\n\n将浮点值截断为整数部分。',
        parameters: [{ label: 'value', documentation: 'The specified input. | 指定的输入' }],
    },

    // ============ Wave Intrinsics (Shader Model 6.0+) ============
    WaveActiveAllEqual: {
        description: 'Returns true if the expression is the same for every active lane in the current wave.\n\n如果当前波中所有活动通道的表达式相同，则返回 true。',
        parameters: [{ label: 'value', documentation: 'The value to compare. | 要比较的值' }],
    },
    WaveActiveBitAnd: {
        description: 'Returns the bitwise AND of all the values of the expression across all active lanes in the current wave.\n\n返回当前波中所有活动通道表达式值的按位与结果。',
        parameters: [{ label: 'value', documentation: 'The value to AND. | 要进行与运算的值' }],
    },
    WaveActiveBitOr: {
        description: 'Returns the bitwise OR of all the values of the expression across all active lanes in the current wave.\n\n返回当前波中所有活动通道表达式值的按位或结果。',
        parameters: [{ label: 'value', documentation: 'The value to OR. | 要进行或运算的值' }],
    },
    WaveActiveBitXor: {
        description: 'Returns the bitwise XOR of all the values of the expression across all active lanes in the current wave.\n\n返回当前波中所有活动通道表达式值的按位异或结果。',
        parameters: [{ label: 'value', documentation: 'The value to XOR. | 要进行异或运算的值' }],
    },
    WaveActiveCountBits: {
        description: 'Counts the number of boolean variables which evaluate to true across all active lanes in the current wave.\n\n统计当前波中所有活动通道中值为 true 的布尔变量数量。',
        parameters: [{ label: 'value', documentation: 'The boolean value to count. | 要统计的布尔值' }],
    },
    WaveActiveMax: {
        description: 'Returns the maximum value of the expression across all active lanes in the current wave.\n\n返回当前波中所有活动通道表达式的最大值。',
        parameters: [{ label: 'value', documentation: 'The value to find the max of. | 要查找最大值的值' }],
    },
    WaveActiveMin: {
        description: 'Returns the minimum value of the expression across all active lanes in the current wave.\n\n返回当前波中所有活动通道表达式的最小值。',
        parameters: [{ label: 'value', documentation: 'The value to find the min of. | 要查找最小值的值' }],
    },
    WaveActiveProduct: {
        description: 'Returns the product of all the values of the expression across all active lanes in the current wave.\n\n返回当前波中所有活动通道表达式值的乘积。',
        parameters: [{ label: 'value', documentation: 'The value to multiply. | 要相乘的值' }],
    },
    WaveActiveSum: {
        description: 'Returns the sum of all the values of the expression across all active lanes in the current wave.\n\n返回当前波中所有活动通道表达式值的总和。',
        parameters: [{ label: 'value', documentation: 'The value to sum. | 要求和的值' }],
    },
    WaveActiveAllTrue: {
        description: 'Returns true if the expression is true in all active lanes in the current wave.\n\n如果当前波中所有活动通道的表达式都为 true，则返回 true。',
        parameters: [{ label: 'value', documentation: 'The boolean value to test. | 要测试的布尔值' }],
    },
    WaveActiveAnyTrue: {
        description: 'Returns true if the expression is true in any active lane in the current wave.\n\n如果当前波中任一活动通道的表达式为 true，则返回 true。',
        parameters: [{ label: 'value', documentation: 'The boolean value to test. | 要测试的布尔值' }],
    },
    WaveActiveBallot: {
        description: 'Returns a uint4 containing a bitmask of the evaluation of the boolean expression for all active lanes in the current wave.\n\n返回一个 uint4，包含当前波中所有活动通道布尔表达式求值的位掩码。',
        parameters: [{ label: 'value', documentation: 'The boolean value to test. | 要测试的布尔值' }],
    },
    WaveGetLaneCount: {
        description: 'Returns the number of lanes in a wave on this architecture.\n\n返回当前架构上波中的通道数。',
    },
    WaveGetLaneIndex: {
        description: 'Returns the index of the current lane within the current wave.\n\n返回当前波中当前通道的索引。',
    },
    WaveIsFirstLane: {
        description: 'Returns true only for the active lane in the current wave with the smallest index.\n\n仅对当前波中索引最小的活动通道返回 true。',
    },
    WavePrefixCountBits: {
        description: 'Returns the sum of all the specified boolean values in active lanes in the current wave with indices less than this lane.\n\n返回当前波中索引小于当前通道的所有活动通道中指定布尔值的总和。',
        parameters: [{ label: 'value', documentation: 'The boolean value to count. | 要统计的布尔值' }],
    },
    WavePrefixProduct: {
        description: 'Returns the product of all of the values in the active lanes in this wave with indices less than this lane.\n\n返回当前波中索引小于当前通道的所有活动通道值的乘积。',
        parameters: [{ label: 'value', documentation: 'The value to multiply. | 要相乘的值' }],
    },
    WavePrefixSum: {
        description: 'Returns the sum of all of the values in the active lanes with smaller indices than this one.\n\n返回当前波中索引小于当前通道的所有活动通道值的总和。',
        parameters: [{ label: 'value', documentation: 'The value to sum. | 要求和的值' }],
    },
    WaveReadLaneAt: {
        description: 'Returns the value of the expression for the given lane index within the specified wave.\n\n返回指定波中给定通道索引处表达式的值。',
        parameters: [
            { label: 'value', documentation: 'The value to read. | 要读取的值' },
            { label: 'laneIndex', documentation: 'The lane index to read from. | 要读取的通道索引' }
        ],
    },
    WaveReadLaneFirst: {
        description: 'Returns the value of the expression for the active lane of the current wave with the smallest index.\n\n返回当前波中索引最小的活动通道的表达式值。',
        parameters: [{ label: 'value', documentation: 'The value to read. | 要读取的值' }],
    },

    // ============ Quad Intrinsics (Shader Model 6.0+) ============
    QuadReadLaneAt: {
        description: 'Returns the specified source value from the lane identified by the lane ID within the current quad.\n\n返回当前四边形中由通道 ID 标识的通道的指定源值。',
        parameters: [
            { label: 'value', documentation: 'The source value. | 源值' },
            { label: 'quadLaneID', documentation: 'The lane ID within the quad (0-3). | 四边形内的通道 ID（0-3）' }
        ],
    },
    QuadReadAcrossDiagonal: {
        description: 'Returns the specified local value which is read from the diagonally opposite lane in this quad.\n\n返回从当前四边形对角线相对通道读取的指定本地值。',
        parameters: [{ label: 'value', documentation: 'The source value. | 源值' }],
    },
    QuadReadAcrossX: {
        description: 'Returns the specified local value read from the other lane in this quad in the X direction.\n\n返回从当前四边形 X 方向另一通道读取的指定本地值。',
        parameters: [{ label: 'value', documentation: 'The source value. | 源值' }],
    },
    QuadReadAcrossY: {
        description: 'Returns the specified local value read from the other lane in this quad in the Y direction.\n\n返回从当前四边形 Y 方向另一通道读取的指定本地值。',
        parameters: [{ label: 'value', documentation: 'The source value. | 源值' }],
    },

    // ============ Other Advanced Functions ============
    NonUniformResourceIndex: {
        description: 'Marks a resource index as non-uniform, allowing divergent indexing into resource arrays.\n\n将资源索引标记为非均匀，允许对资源数组进行发散索引。',
        parameters: [{ label: 'index', documentation: 'The resource index. | 资源索引' }],
    },
    
    // ============ Texture Object Methods ============
    Sample: {
        description: 'Samples a texture.\n\n采样纹理。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler state. | 采样器状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' }
        ],
    },
    SampleBias: {
        description: 'Samples a texture with a bias applied to the mipmap level.\n\n采样纹理，并对 mipmap 级别应用偏置。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler state. | 采样器状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' },
            { label: 'bias', documentation: 'The bias value. | 偏置值' }
        ],
    },
    SampleCmp: {
        description: 'Samples a texture and compares a single component against the specified comparison value.\n\n采样纹理并将单个分量与指定的比较值进行比较。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler comparison state. | 采样器比较状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' },
            { label: 'compareValue', documentation: 'A value to compare. | 要比较的值' }
        ],
    },
    SampleCmpLevelZero: {
        description: 'Samples a texture at mipmap level 0 and compares the result to a comparison value.\n\n在 mipmap 级别 0 采样纹理，并将结果与比较值进行比较。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler comparison state. | 采样器比较状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' },
            { label: 'compareValue', documentation: 'A value to compare. | 要比较的值' }
        ],
    },
    SampleGrad: {
        description: 'Samples a texture using a gradient to influence the way the sample location is calculated.\n\n采样纹理，使用梯度影响采样位置的计算方式。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler state. | 采样器状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' },
            { label: 'ddx', documentation: 'The rate of change in the x direction. | x 方向的变化率' },
            { label: 'ddy', documentation: 'The rate of change in the y direction. | y 方向的变化率' }
        ],
    },
    SampleLevel: {
        description: 'Samples a texture on the specified mipmap level.\n\n在指定的 mipmap 级别采样纹理。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler state. | 采样器状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' },
            { label: 'lod', documentation: 'The mipmap level. | mipmap 级别' }
        ],
    },
    Load: {
        description: 'Reads texel data without any filtering or sampling.\n\n读取纹素数据，不进行任何过滤或采样。',
        parameters: [{ label: 'location', documentation: 'The texture coordinates with mip level. | 带 mip 级别的纹理坐标' }],
    },
    Gather: {
        description: 'Gets the four samples (red component only) that would be used for bilinear interpolation when sampling a texture.\n\n获取采样纹理时用于双线性插值的四个样本（仅红色分量）。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler state. | 采样器状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' }
        ],
    },
    GatherRed: {
        description: 'Gets the red components of the four samples that would be used for bilinear interpolation.\n\n获取用于双线性插值的四个样本的红色分量。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler state. | 采样器状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' }
        ],
    },
    GatherGreen: {
        description: 'Gets the green components of the four samples that would be used for bilinear interpolation.\n\n获取用于双线性插值的四个样本的绿色分量。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler state. | 采样器状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' }
        ],
    },
    GatherBlue: {
        description: 'Gets the blue components of the four samples that would be used for bilinear interpolation.\n\n获取用于双线性插值的四个样本的蓝色分量。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler state. | 采样器状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' }
        ],
    },
    GatherAlpha: {
        description: 'Gets the alpha components of the four samples that would be used for bilinear interpolation.\n\n获取用于双线性插值的四个样本的 Alpha 分量。',
        parameters: [
            { label: 'sampler', documentation: 'The sampler state. | 采样器状态' },
            { label: 'location', documentation: 'The texture coordinates. | 纹理坐标' }
        ],
    },
    GetDimensions: {
        description: 'Gets the texture dimensions.\n\n获取纹理尺寸。',
        parameters: [
            { label: 'width', documentation: 'The texture width. | 纹理宽度' },
            { label: 'height', documentation: 'The texture height (if applicable). | 纹理高度（如适用）' },
            { label: 'numberOfLevels', documentation: 'The number of mipmap levels. | mipmap 级别数' }
        ],
    }
};

export var preprocessors: IEntries = {
    DEFINE: {
        description: 'Preprocessor directive that defines a constant or a macro.\n\n预处理器指令，用于定义常量或宏。',
    },
    ERROR: {
        description: 'Preprocessor directive that produces compiler-time error messages.\n\n预处理器指令，用于生成编译时错误消息。',
    },
    IF: {
        description: 'Preprocessor directives that control compilation of portions of a source file.\n\n预处理器指令，用于控制源文件部分内容的编译。',
    },
    ELIF: {
        description: 'Preprocessor directives that control compilation of portions of a source file.\n\n预处理器指令，用于控制源文件部分内容的编译。',
    },
    ELSE: {
        description: 'Preprocessor directives that control compilation of portions of a source file.\n\n预处理器指令，用于控制源文件部分内容的编译。',
    },
    ENDIF: {
        description: 'Preprocessor directives that control compilation of portions of a source file.\n\n预处理器指令，用于控制源文件部分内容的编译。',
    },
    IFDEF: {
        description: 'Preprocessor directives that determine whether a specific preprocessor constant or macro is defined.\n\n预处理器指令，用于判断特定的预处理器常量或宏是否已定义。',
    },
    IFNDEF: {
        description: 'Preprocessor directives that determine whether a specific preprocessor constant or macro is defined.\n\n预处理器指令，用于判断特定的预处理器常量或宏是否未定义。',
    },
    INCLUDE: {
        description: 'Preprocessor directive that inserts the contents of the specified file into the source program at the point where the directive appears.\n\n预处理器指令，用于在指令出现的位置将指定文件的内容插入到源程序中。',
    },
    LINE: {
        description: "Preprocessor directive that sets the compiler's internally-stored line number and filename to the specified values.\n\n预处理器指令，用于将编译器内部存储的行号和文件名设置为指定值。",
    },
    PRAGMA: {
        description: 'Preprocessor directive that provides machine-specific or operating system-specific features while retaining overall compatibility with the C and C++ languages.\n\n预处理器指令，用于提供特定于机器或操作系统的功能，同时保持与 C 和 C++ 语言的整体兼容性。',
    },
    UNDEF: {
        description: 'Preprocessor directive that removes the current definition of a constant or macro that was previously defined using the #define directive.\n\n预处理器指令，用于移除之前使用 #define 指令定义的常量或宏的当前定义。',
    }
};

export var semanticsNum: IEntries = {
    BINORMAL: {
        description: 'Binormal',
    },
    BLENDINDICES: {
        description: 'Blend indices',
    },
    BLENDWEIGHT: {
        description: 'Blend weights',
    },
    COLOR: {
        description: 'Diffuse and/or specular color',
    },
    NORMAL: {
        description: 'Normal vector',
    },
    POSITION: {
        description: 'Vertex position in object space.',
    },
    PSIZE: {
        description: 'Point size',
    },
    TANGENT: {
        description: 'Tangent',
    },
    TEXCOORD: {
        description: 'Texture coordinates',
    },
    TESSFACTOR: {
        description: 'Tessellation factor',
    },
    DEPTH: {
        description: 'Output depth',
    },
    SV_CLIPDISTANCE: {
        description: 'Clip distance data.',
    },
    SV_CULLDISTANCE: {
        description: 'Cull distance data.',
    },
    SV_DEPTHGREATEREQUAL: {
        description: 'Valid in any shader, tests whether the value is greater than or equal to the depth data value.',
    },
    SV_DEPTHLESSEQUAL: {
        description: 'Valid in any shader, tests whether the value is less than or equal to the depth data value.',
    },
    SV_TARGET: {
        description: 'The output value that will be stored in a render target. The index indicates which of the 8 possibly bound render targets to write to. The value is available to all shaders.',
    }
};

export var semantics: IEntries = {
    POSITIONT: {
        description: 'Transformed vertex position.',
    },
    FOG: {
        description: 'Vertex fog',
    },
    PSIZE: {
        description: 'Point size',
    },
    VFACE: {
        description: 'Floating-point scalar that indicates a back-facing primitive. A negative value faces backwards, while a positive value faces the camera.',
    },
    VPOS: {
        description: 'The pixel location (x,y) in screen space.',
    },
    SV_COVERAGE: {
        description: 'A mask that can be specified on input, output, or both of a pixel shader.',
    },
    SV_DEPTH: {
        description: 'Depth buffer data. Can be written or read by any shader.',
    },
    SV_DISPATCHTHREADID: {
        description: 'Defines the global thread offset within the Dispatch call, per dimension of the group. Available as input to compute shader. (read only)',
    },
    SV_DOMAINLOCATION: {
        description: 'Defines the location on the hull of the current domain point being evaluated. Available as input to the domain shader. (read only)',
    },
    SV_GROUPID: {
        description: 'Defines the group offset within a Dispatch call, per dimension of the dispatch call. Available as input to the compute shader. (read only)',
    },
    SV_GROUPINDEX: {
        description: 'Provides a flattened index for a given thread within a given group. Available as input to the compute shader. (read only)',
    },
    SV_GROUPTHREADID: {
        description: 'Defines the thread offset within the group, per dimension of the group. Available as input to the compute shader. (read only)',
    },
    SV_GSINSTANCEID: {
        description: 'Defines the instance of the geometry shader. Available as input to the geometry shader. The instance is needed as a geometry shader can be invoked up to 32 times on the same geometry primitive.',
    },
    SV_INNERCOVERAGE: {
        description: 'Represents underestimated conservative rasterization information (i.e. whether a pixel is guaranteed-to-be-fully covered). Can be read or written by the pixel shader.',
    },
    SV_INSIDETESSFACTOR: {
        description: 'Defines the tessellation amount within a patch surface. Available in the hull shader for writing, and available in the domain shader for reading.',
    },
    SV_INSTANCEID: {
        description: 'Per-instance identifier automatically generated by the runtime (see Using System-Generated Values (Direct3D 10)). Available to all shaders.',
    },
    SV_ISFRONTFACE: {
        description: 'Specifies whether a triangle is front facing. For lines and points, IsFrontFace has the value true. The exception is lines drawn out of triangles (wireframe mode), which sets IsFrontFace the same way as rasterizing the triangle in solid mode. Can be written to by the geometry shader, and read by the pixel shader.',
    },
    SV_OUTPUTCONTROLPOINTID: {
        description: 'Defines the index of the control point ID being operated on by an invocation of the main entry point of the hull shader. Can be read by the hull shader only.',
    },
    SV_POSITION: {
        description: 'When SV_Position is declared for input to a shader, it can have one of two interpolation modes specified: linearNoPerspective or linearNoPerspectiveCentroid, where the latter causes centroid-snapped xyzw values to be provided when multisample antialiasing. When used in a shader, SV_Position describes the pixel location. Available in all shaders to get the pixel center with a 0.5 offset.',
    },
    SV_PRIMITIVEID: {
        description: 'Per-primitive identifier automatically generated by the runtime (see Using System-Generated Values (Direct3D 10)). Can be written to by the geometry or pixel shaders, and read by the geometry, pixel, hull or domain shaders.',
    },
    SV_RENDERTARGETARRAYINDEX: {
        description: 'Render-target array index. Applied to geometry shader output and indicates the render target array slice that the primitive will be drawn to by the pixel shader. SV_RenderTargetArrayIndex is only valid if the render target is an array resource. This semantic applies only to primitives, if a primitive has more than one vertex the value from the leading vertex will be used. This value also indicates which array slice of a depthstencilview is used for read/write purposes. Can be written from the geometry shader and read or written by the pixel shader.',
    },
    SV_SAMPLEINDEX: {
        description: 'Sample frequency index data. Available to be read or written to by the pixel shader only.',
    },
    SV_STENCILREF: {
        description: 'Represents the current pixel shader stencil reference value. Can be written by the pixel shader only.',
    },
    SV_TESSFACTOR: {
        description: 'Defines the tessellation amount on each edge of a patch. Available for writing in the hull shader and reading in the domain shader.',
    },
    SV_VERTEXID: {
        description: 'Per-vertex identifier automatically generated by the runtime (see Using System-Generated Values (Direct3D 10)). Available as the input to the vertex shader only.',
    },
    SV_VIEWPORTARRAYINDEX: {
        description: 'Viewport array index. Applied to geometry shader output and indicates which viewport to use for the primitive currently being written out. Can be read or written by the pixel shader. The primitive will be transformed and clipped against the viewport specified by the index before it is passed to the rasterizer. This semantic applies only to primitives, if a primitive has more than one vertex the value from the leading vertex will be used.',
    }
};

export var datatypes: IEntries = {
    bool: {
        description: 'true or false.',
    },
    int: {
        description: '32-bit signed integer.',
    },
    int1: {
        description: '32-bit signed integer vector.',
    },
    int2: {
        description: '32-bit signed integer vector.',
    },
    int3: {
        description: '32-bit signed integer vector.',
    },
    int4: {
        description: '32-bit signed integer vector.',
    },
    int1x1: {
        description: '32-bit signed integer matrix.',
    },
    int1x2: {
        description: '32-bit signed integer matrix.',
    },
    int1x3: {
        description: '32-bit signed integer matrix.',
    },
    int1x4: {
        description: '32-bit signed integer matrix.',
    },
    int2x1: {
        description: '32-bit signed integer matrix.',
    },
    int2x2: {
        description: '32-bit signed integer matrix.',
    },
    int2x3: {
        description: '32-bit signed integer matrix.',
    },
    int2x4: {
        description: '32-bit signed integer matrix.',
    },
    int3x1: {
        description: '32-bit signed integer matrix.',
    },
    int3x2: {
        description: '32-bit signed integer matrix.',
    },
    int3x3: {
        description: '32-bit signed integer matrix.',
    },
    int3x4: {
        description: '32-bit signed integer matrix.',
    },
    int4x1: {
        description: '32-bit signed integer matrix.',
    },
    int4x2: {
        description: '32-bit signed integer matrix.',
    },
    int4x3: {
        description: '32-bit signed integer matrix.',
    },
    int4x4: {
        description: '32-bit signed integer matrix.',
    },
    uint: {
        description: '32-bit unsigned integer.',
    },
    dword: {
        description: '32-bit unsigned integer.',
    },
    half: {
        description: '16-bit floating point value. This data type is provided only for language compatibility. Direct3D 10 shader targets map all half data types to float data types. A half data type cannot be used on a uniform global variable (use the /Gec flag if this functionality is desired).',
    },
    half1: {
        description: '16-bit floating point vector.',
    },
    half2: {
        description: '16-bit floating point vector.',
    },
    half3: {
        description: '16-bit floating point vector.',
    },
    half4: {
        description: '16-bit floating point vector.',
    },
    float: {
        description: '32-bit floating point value.',
    },
    float1: {
        description: '32-bit floating point vector.',
    },
    float2: {
        description: '32-bit floating point vector.',
    },
    float3: {
        description: '32-bit floating point vector.',
    },
    float4: {
        description: '32-bit floating point vector.',
    },
    float1x1: {
        description: '32-bit floating point matrix.',
    },
    float1x2: {
        description: '32-bit floating point matrix.',
    },
    float1x3: {
        description: '32-bit floating point matrix.',
    },
    float1x4: {
        description: '32-bit floating point matrix.',
    },
    float2x1: {
        description: '32-bit floating point matrix.',
    },
    float2x2: {
        description: '32-bit floating point matrix.',
    },
    float2x3: {
        description: '32-bit floating point matrix.',
    },
    float2x4: {
        description: '32-bit floating point matrix.',
    },
    float3x1: {
        description: '32-bit floating point matrix.',
    },
    float3x2: {
        description: '32-bit floating point matrix.',
    },
    float3x3: {
        description: '32-bit floating point matrix.',
    },
    float3x4: {
        description: '32-bit floating point matrix.',
    },
    float4x1: {
        description: '32-bit floating point matrix.',
    },
    float4x2: {
        description: '32-bit floating point matrix.',
    },
    float4x3: {
        description: '32-bit floating point matrix.',
    },
    float4x4: {
        description: '32-bit floating point matrix.',
    },
    double: {
        description: '64-bit floating point value. You cannot use double precision values as inputs and outputs for a stream. To pass double precision values between shaders, declare each double as a pair of uint data types. Then, use the asdouble function to pack each double into the pair of uints and the asuint function to unpack the pair of uints back into the double.',
    },
    min16float: {
        description: 'minimum 16-bit floating point value.',
    },
    min10float: {
        description: 'minimum 10-bit floating point value.',
    },
    min16int: {
        description: 'minimum 16-bit signed integer.',
    },
    min12int: {
        description: 'minimum 12-bit signed integer.',
    },
    min16uint: {
        description: 'minimum 16-bit unsigned integer.',
    },
    Buffer: {
        description: 'Use to declare a buffer variable.',
    },
    vector: {
        description: 'A vector contains between one and four scalar components; every component of a vector must be of the same type.',
    },
    matrix: {
        description: 'A matrix is a special data type that contains between one and sixteen components. Every component of a matrix must be of the same type.',
    },
    sampler: {
        description: 'Use to declare sampler state as well as sampler-comparison state.',
    },
    SamplerState: {
        description: 'Use to declare sampler state as well as sampler-comparison state.',
    },
    PixelShader: {
        description: 'Declare a shader variable within an effect pass.',
    },
    VertexShader: {
        description: 'Declare a shader variable within an effect pass.',
    },
    texture: {
        description: 'Use to declare a texture variable.',
    },
    Texture1D: {
        description: 'Use to declare a texture variable.',
    },
    Texture1DArray: {
        description: 'Use to declare a texture variable.',
    },
    Texture2D: {
        description: 'Use to declare a texture variable.',
    },
    Texture2DArray: {
        description: 'Use to declare a texture variable.',
    },
    Texture2DMS: {
        description: 'Use to declare a texture variable.',
    },
    Texture2DMSArray: {
        description: 'Use to declare a texture variable.',
    },
    Texture3D: {
        description: 'Use to declare a texture variable.',
    },
    TextureCube: {
        description: 'Use to declare a texture variable.',
    },
    TextureCubeArray: {
        description: 'Use to declare a texture variable.',
    },
    struct: {
        description: 'Use to declare a structure using HLSL.',
    },
    typedef: {
        description: 'Use to declare a user-defined type.',
    }
};

//TODO: descriptions and links
export var keywords: IEntries = {
    AppendStructuredBuffer: { description: "" },
    asm: { description: "" },
    asm_fragment: { description: "" },
    BlendState: { description: "" }, 
    bool: { description: "" }, 
    break: { description: "" }, 
    Buffer: { description: "" }, 
    ByteAddressBuffer: { description: "" },
    case: { description: "" }, 
    cbuffer: { description: "" }, 
    centroid: { description: "" }, 
    class: { description: "" }, 
    column_major: { description: "" }, 
    compile: { description: "" }, 
    compile_fragment: { description: "" }, 
    CompileShader: { description: "" }, 
    const: { description: "" }, 
    continue: { description: "" }, 
    ComputeShader: { description: "" }, 
    ConsumeStructuredBuffer: { description: "" },
    default: { description: "" }, 
    DepthStencilState: { description: "" }, 
    DepthStencilView: { description: "" }, 
    discard: { description: "" }, 
    do: { description: "" }, 
    double: { description: "" }, 
    DomainShader: { description: "" }, 
    dword: { description: "" },
    else: { description: "" }, 
    export: { description: "" }, 
    extern: { description: "" },
    false: { description: "" }, 
    float: { description: "" }, 
    for: { description: "" }, 
    fxgroup: { description: "" },
    GeometryShader: { description: "" }, 
    groupshared: { description: "" },
    half: { description: "" }, 
    Hullshader: { description: "" },
    if: { description: "" }, 
    in: { description: "" }, 
    inline: { description: "" }, 
    inout: { description: "" }, 
    InputPatch: { description: "" }, 
    int: { description: "" }, 
    interface: { description: "" },
    line: { description: "" }, 
    lineadj: { description: "" }, 
    linear: { description: "" }, 
    LineStream: { description: "" },
    matrix: { description: "" }, 
    min16float: { description: "" }, 
    min10float: { description: "" }, 
    min16int: { description: "" }, 
    min12int: { description: "" }, 
    min16uint: { description: "" },
    namespace: { description: "" }, 
    nointerpolation: { description: "" }, 
    noperspective: { description: "" }, 
    NULL: { description: "" },
    out: { description: "" }, 
    OutputPatch: { description: "" },
    packoffset: { description: "" }, 
    pass: { description: "" }, 
    pixelfragment: { description: "" }, 
    PixelShader: { description: "" }, 
    point: { description: "" }, 
    PointStream: { description: "" }, 
    precise: { description: "" },
    RasterizerState: { description: "" }, 
    RenderTargetView: { description: "" }, 
    return: { description: "" }, 
    register: { description: "" }, 
    row_major: { description: "" }, 
    RWBuffer: { description: "" }, 
    RWByteAddressBuffer: { description: "" }, 
    RWStructuredBuffer: { description: "" }, 
    RWTexture1D: { description: "" }, 
    RWTexture1DArray: { description: "" }, 
    RWTexture2D: { description: "" }, 
    RWTexture2DArray: { description: "" }, 
    RWTexture3D: { description: "" },
    sample: { description: "" }, 
    sampler: { description: "" }, 
    SamplerState: { description: "" }, 
    SamplerComparisonState: { description: "" }, 
    shared: { description: "" }, 
    snorm: { description: "" }, 
    stateblock: { description: "" }, 
    stateblock_state: { description: "" }, 
    static: { description: "" }, 
    string: { description: "" }, 
    struct: { description: "" }, 
    switch: { description: "" }, 
    StructuredBuffer: { description: "" },
    tbuffer: { description: "" }, 
    technique: { description: "" }, 
    technique10: { description: "" }, 
    technique11: { description: "" }, 
    texture: { description: "" }, 
    Texture1D: { description: "" }, 
    Texture1DArray: { description: "" }, 
    Texture2D: { description: "" }, 
    Texture2DArray: { description: "" }, 
    Texture2DMS: { description: "" }, 
    Texture2DMSArray: { description: "" }, 
    Texture3D: { description: "" }, 
    TextureCube: { description: "" }, 
    TextureCubeArray: { description: "" }, 
    true: { description: "" }, 
    typedef: { description: "" }, 
    triangle: { description: "" }, 
    triangleadj: { description: "" }, 
    TriangleStream: { description: "" },
    uint: { description: "" }, 
    uniform: { description: "" }, 
    unorm: { description: "" }, 
    unsigned: { description: "" },
    vector: { description: "" }, 
    vertexfragment: { description: "" }, 
    VertexShader: { description: "" }, 
    void: { description: "" }, 
    volatile: { description: "" },
    while: { description: "" }
};


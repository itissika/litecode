/**
 * Vendored Unity ShaderLab / Built-in HLSL tables from
 * https://github.com/yyqrc/vscode-unitylabshader (MIT).
 *
 * Unity Shader 内置全局定义
 * 包含 Unity 内置变量、函数、宏的定义用于代码补全和悬停提示
 */

// ============================================================================
// Unity 内置变量定义
// ============================================================================

export interface UnityVariable {
    name: string;
    type: string;
    description: string;
    category: string;
}

export const unityBuiltinVariables: UnityVariable[] = [
    // 变换矩阵
    { name: 'UNITY_MATRIX_MVP', type: 'float4x4', description: '当前模型视图投影矩阵', category: 'Matrix' },
    { name: 'UNITY_MATRIX_MV', type: 'float4x4', description: '当前模型视图矩阵', category: 'Matrix' },
    { name: 'UNITY_MATRIX_V', type: 'float4x4', description: '当前视图矩阵', category: 'Matrix' },
    { name: 'UNITY_MATRIX_P', type: 'float4x4', description: '当前投影矩阵', category: 'Matrix' },
    { name: 'UNITY_MATRIX_VP', type: 'float4x4', description: '当前视图投影矩阵', category: 'Matrix' },
    { name: 'UNITY_MATRIX_T_MV', type: 'float4x4', description: '模型视图矩阵的转置', category: 'Matrix' },
    { name: 'UNITY_MATRIX_IT_MV', type: 'float4x4', description: '模型视图矩阵的逆转置（用于法线变换）', category: 'Matrix' },
    { name: 'UNITY_MATRIX_M', type: 'float4x4', description: '当前模型矩阵', category: 'Matrix' },
    { name: 'UNITY_MATRIX_I_M', type: 'float4x4', description: '当前模型矩阵的逆矩阵', category: 'Matrix' },
    { name: 'UNITY_MATRIX_I_V', type: 'float4x4', description: '当前视图矩阵的逆矩阵', category: 'Matrix' },
    { name: 'unity_ObjectToWorld', type: 'float4x4', description: '模型空间到世界空间的变换矩阵', category: 'Matrix' },
    { name: 'unity_WorldToObject', type: 'float4x4', description: '世界空间到模型空间的变换矩阵', category: 'Matrix' },
    { name: 'unity_MatrixVP', type: 'float4x4', description: '视图投影矩阵', category: 'Matrix' },
    { name: 'unity_MatrixV', type: 'float4x4', description: '视图矩阵', category: 'Matrix' },
    { name: 'unity_MatrixInvV', type: 'float4x4', description: '视图矩阵的逆矩阵', category: 'Matrix' },

    // 相机参数
    { name: '_WorldSpaceCameraPos', type: 'float3', description: '相机的世界空间位置', category: 'Camera' },
    { name: '_ProjectionParams', type: 'float4', description: '投影参数: x=1.0/-1.0, y=near, z=far, w=1/far', category: 'Camera' },
    { name: '_ScreenParams', type: 'float4', description: '屏幕参数: x=width, y=height, z=1+1/width, w=1+1/height', category: 'Camera' },
    { name: '_ZBufferParams', type: 'float4', description: 'Z缓冲参数，用于线性化深度', category: 'Camera' },
    { name: 'unity_OrthoParams', type: 'float4', description: '正交相机参数: x=width, y=height, z=unused, w=1(正交)/0(透视)', category: 'Camera' },
    { name: 'unity_CameraProjection', type: 'float4x4', description: '相机投影矩阵', category: 'Camera' },
    { name: 'unity_CameraInvProjection', type: 'float4x4', description: '相机投影矩阵的逆矩阵', category: 'Camera' },
    { name: 'unity_CameraWorldClipPlanes', type: 'float4[6]', description: '相机的世界空间裁剪平面', category: 'Camera' },

    // 时间变量
    { name: '_Time', type: 'float4', description: '时间变量: (t/20, t, t*2, t*3)', category: 'Time' },
    { name: '_SinTime', type: 'float4', description: 'sin(时间): (t/8, t/4, t/2, t)', category: 'Time' },
    { name: '_CosTime', type: 'float4', description: 'cos(时间): (t/8, t/4, t/2, t)', category: 'Time' },
    { name: 'unity_DeltaTime', type: 'float4', description: '增量时间: (dt, 1/dt, smoothDt, 1/smoothDt)', category: 'Time' },

    // 光照变量
    { name: '_WorldSpaceLightPos0', type: 'float4', description: '主光源的世界空间位置/方向（w=0为方向光，w=1为点光源）', category: 'Lighting' },
    { name: '_LightPositionRange', type: 'float4', description: '光源位置和范围', category: 'Lighting' },
    { name: '_LightColor0', type: 'fixed4', description: '主光源颜色', category: 'Lighting' },
    { name: 'unity_4LightPosX0', type: 'float4', description: '前4个非重要点光源的X坐标', category: 'Lighting' },
    { name: 'unity_4LightPosY0', type: 'float4', description: '前4个非重要点光源的Y坐标', category: 'Lighting' },
    { name: 'unity_4LightPosZ0', type: 'float4', description: '前4个非重要点光源的Z坐标', category: 'Lighting' },
    { name: 'unity_4LightAtten0', type: 'float4', description: '前4个非重要点光源的衰减', category: 'Lighting' },
    { name: 'unity_LightColor', type: 'half4[4]', description: '前4个非重要点光源的颜色', category: 'Lighting' },
    { name: 'unity_WorldToShadow', type: 'float4x4[4]', description: '世界空间到阴影空间的变换矩阵', category: 'Lighting' },
    { name: '_LightShadowData', type: 'float4', description: '阴影数据', category: 'Lighting' },
    { name: 'unity_ShadowFadeCenterAndType', type: 'float4', description: '阴影淡出中心和类型', category: 'Lighting' },

    // 环境光/球谐
    { name: 'unity_AmbientSky', type: 'fixed4', description: '天空环境光颜色', category: 'Ambient' },
    { name: 'unity_AmbientEquator', type: 'fixed4', description: '赤道环境光颜色', category: 'Ambient' },
    { name: 'unity_AmbientGround', type: 'fixed4', description: '地面环境光颜色', category: 'Ambient' },
    { name: 'UNITY_LIGHTMODEL_AMBIENT', type: 'fixed4', description: '环境光颜色（旧版）', category: 'Ambient' },
    { name: 'unity_SHAr', type: 'half4', description: '球谐系数 Ar', category: 'Ambient' },
    { name: 'unity_SHAg', type: 'half4', description: '球谐系数 Ag', category: 'Ambient' },
    { name: 'unity_SHAb', type: 'half4', description: '球谐系数 Ab', category: 'Ambient' },
    { name: 'unity_SHBr', type: 'half4', description: '球谐系数 Br', category: 'Ambient' },
    { name: 'unity_SHBg', type: 'half4', description: '球谐系数 Bg', category: 'Ambient' },
    { name: 'unity_SHBb', type: 'half4', description: '球谐系数 Bb', category: 'Ambient' },
    { name: 'unity_SHC', type: 'half4', description: '球谐系数 C', category: 'Ambient' },

    // 雾效
    { name: 'unity_FogColor', type: 'fixed4', description: '雾颜色', category: 'Fog' },
    { name: 'unity_FogParams', type: 'float4', description: '雾参数: (density/sqrt(ln(2)), density/ln(2), -1/(end-start), end/(end-start))', category: 'Fog' },

    // 光照贴图
    { name: 'unity_Lightmap', type: 'sampler2D', description: '光照贴图', category: 'Lightmap' },
    { name: 'unity_LightmapST', type: 'float4', description: '光照贴图的缩放和偏移', category: 'Lightmap' },
    { name: 'unity_DynamicLightmap', type: 'sampler2D', description: '动态光照贴图', category: 'Lightmap' },
    { name: 'unity_DynamicLightmapST', type: 'float4', description: '动态光照贴图的缩放和偏移', category: 'Lightmap' },
];

// ============================================================================
// Unity 内置函数定义
// ============================================================================

export interface UnityFunction {
    name: string;
    signature: string;
    description: string;
    parameters: { name: string; type: string; description: string }[];
    returnType: string;
    category: string;
}

export const unityBuiltinFunctions: UnityFunction[] = [
    // 空间变换函数
    {
        name: 'UnityObjectToClipPos',
        signature: 'float4 UnityObjectToClipPos(float3 pos)',
        description: '将顶点从模型空间变换到裁剪空间',
        parameters: [{ name: 'pos', type: 'float3', description: '模型空间顶点位置' }],
        returnType: 'float4',
        category: 'Transform'
    },
    {
        name: 'UnityObjectToViewPos',
        signature: 'float3 UnityObjectToViewPos(float3 pos)',
        description: '将顶点从模型空间变换到视图空间',
        parameters: [{ name: 'pos', type: 'float3', description: '模型空间顶点位置' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'UnityWorldToClipPos',
        signature: 'float4 UnityWorldToClipPos(float3 pos)',
        description: '将顶点从世界空间变换到裁剪空间',
        parameters: [{ name: 'pos', type: 'float3', description: '世界空间顶点位置' }],
        returnType: 'float4',
        category: 'Transform'
    },
    {
        name: 'UnityViewToClipPos',
        signature: 'float4 UnityViewToClipPos(float3 pos)',
        description: '将顶点从视图空间变换到裁剪空间',
        parameters: [{ name: 'pos', type: 'float3', description: '视图空间顶点位置' }],
        returnType: 'float4',
        category: 'Transform'
    },
    {
        name: 'UnityObjectToWorldNormal',
        signature: 'float3 UnityObjectToWorldNormal(float3 norm)',
        description: '将法线从模型空间变换到世界空间',
        parameters: [{ name: 'norm', type: 'float3', description: '模型空间法线' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'UnityObjectToWorldDir',
        signature: 'float3 UnityObjectToWorldDir(float3 dir)',
        description: '将方向向量从模型空间变换到世界空间',
        parameters: [{ name: 'dir', type: 'float3', description: '模型空间方向' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'UnityWorldToObjectDir',
        signature: 'float3 UnityWorldToObjectDir(float3 dir)',
        description: '将方向向量从世界空间变换到模型空间',
        parameters: [{ name: 'dir', type: 'float3', description: '世界空间方向' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'UnityWorldSpaceViewDir',
        signature: 'float3 UnityWorldSpaceViewDir(float4 worldPos)',
        description: '计算从顶点到相机的世界空间方向（未归一化）',
        parameters: [{ name: 'worldPos', type: 'float4', description: '世界空间顶点位置' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'UnityWorldSpaceLightDir',
        signature: 'float3 UnityWorldSpaceLightDir(float4 worldPos)',
        description: '计算从顶点到光源的世界空间方向（未归一化）',
        parameters: [{ name: 'worldPos', type: 'float4', description: '世界空间顶点位置' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'ObjSpaceViewDir',
        signature: 'float3 ObjSpaceViewDir(float4 v)',
        description: '计算从顶点到相机的模型空间方向',
        parameters: [{ name: 'v', type: 'float4', description: '模型空间顶点位置' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'ObjSpaceLightDir',
        signature: 'float3 ObjSpaceLightDir(float4 v)',
        description: '计算从顶点到光源的模型空间方向',
        parameters: [{ name: 'v', type: 'float4', description: '模型空间顶点位置' }],
        returnType: 'float3',
        category: 'Transform'
    },

    // 屏幕坐标函数
    {
        name: 'ComputeScreenPos',
        signature: 'float4 ComputeScreenPos(float4 pos)',
        description: '计算屏幕空间位置（用于屏幕空间效果）',
        parameters: [{ name: 'pos', type: 'float4', description: '裁剪空间位置' }],
        returnType: 'float4',
        category: 'Screen'
    },
    {
        name: 'ComputeGrabScreenPos',
        signature: 'float4 ComputeGrabScreenPos(float4 pos)',
        description: '计算抓取纹理的屏幕坐标',
        parameters: [{ name: 'pos', type: 'float4', description: '裁剪空间位置' }],
        returnType: 'float4',
        category: 'Screen'
    },

    // 深度函数
    {
        name: 'LinearEyeDepth',
        signature: 'float LinearEyeDepth(float z)',
        description: '将深度缓冲值转换为线性视图空间深度',
        parameters: [{ name: 'z', type: 'float', description: '深度缓冲值' }],
        returnType: 'float',
        category: 'Depth'
    },
    {
        name: 'Linear01Depth',
        signature: 'float Linear01Depth(float z)',
        description: '将深度缓冲值转换为0-1范围的线性深度',
        parameters: [{ name: 'z', type: 'float', description: '深度缓冲值' }],
        returnType: 'float',
        category: 'Depth'
    },
    {
        name: 'COMPUTE_EYEDEPTH',
        signature: 'float COMPUTE_EYEDEPTH(float4 pos)',
        description: '计算视图空间深度（宏）',
        parameters: [{ name: 'pos', type: 'float4', description: '视图空间位置' }],
        returnType: 'float',
        category: 'Depth'
    },
    {
        name: 'DECODE_EYEDEPTH',
        signature: 'float DECODE_EYEDEPTH(float z)',
        description: '解码视图空间深度（宏）',
        parameters: [{ name: 'z', type: 'float', description: '深度缓冲值' }],
        returnType: 'float',
        category: 'Depth'
    },

    // 光照函数
    {
        name: 'Shade4PointLights',
        signature: 'float3 Shade4PointLights(...)',
        description: '计算4个点光源的光照',
        parameters: [],
        returnType: 'float3',
        category: 'Lighting'
    },
    {
        name: 'ShadeSH9',
        signature: 'half3 ShadeSH9(half4 normal)',
        description: '使用球谐函数计算环境光照',
        parameters: [{ name: 'normal', type: 'half4', description: '世界空间法线(w=1)' }],
        returnType: 'half3',
        category: 'Lighting'
    },
    {
        name: 'ShadeSH3Order',
        signature: 'half3 ShadeSH3Order(half4 normal)',
        description: '使用3阶球谐函数计算环境光照',
        parameters: [{ name: 'normal', type: 'half4', description: '世界空间法线(w=1)' }],
        returnType: 'half3',
        category: 'Lighting'
    },
    {
        name: 'ShadeVertexLights',
        signature: 'float3 ShadeVertexLights(float4 vertex, float3 normal)',
        description: '在顶点着色器中计算顶点光照',
        parameters: [
            { name: 'vertex', type: 'float4', description: '模型空间顶点位置' },
            { name: 'normal', type: 'float3', description: '模型空间法线' }
        ],
        returnType: 'float3',
        category: 'Lighting'
    },

    // 纹理采样函数
    {
        name: 'tex2D',
        signature: 'half4 tex2D(sampler2D s, float2 uv)',
        description: '2D纹理采样',
        parameters: [
            { name: 's', type: 'sampler2D', description: '纹理采样器' },
            { name: 'uv', type: 'float2', description: '纹理坐标' }
        ],
        returnType: 'half4',
        category: 'Texture'
    },
    {
        name: 'tex2Dlod',
        signature: 'half4 tex2Dlod(sampler2D s, float4 uv)',
        description: '2D纹理采样（指定LOD级别）',
        parameters: [
            { name: 's', type: 'sampler2D', description: '纹理采样器' },
            { name: 'uv', type: 'float4', description: '纹理坐标(xy=uv, w=lod)' }
        ],
        returnType: 'half4',
        category: 'Texture'
    },
    {
        name: 'tex2Dproj',
        signature: 'half4 tex2Dproj(sampler2D s, float4 uv)',
        description: '2D纹理投影采样',
        parameters: [
            { name: 's', type: 'sampler2D', description: '纹理采样器' },
            { name: 'uv', type: 'float4', description: '投影纹理坐标' }
        ],
        returnType: 'half4',
        category: 'Texture'
    },
    {
        name: 'texCUBE',
        signature: 'half4 texCUBE(samplerCUBE s, float3 dir)',
        description: '立方体纹理采样',
        parameters: [
            { name: 's', type: 'samplerCUBE', description: '立方体纹理采样器' },
            { name: 'dir', type: 'float3', description: '采样方向' }
        ],
        returnType: 'half4',
        category: 'Texture'
    },

    // 颜色空间转换
    {
        name: 'GammaToLinearSpace',
        signature: 'half3 GammaToLinearSpace(half3 sRGB)',
        description: '将颜色从 Gamma 空间转换到线性空间',
        parameters: [{ name: 'sRGB', type: 'half3', description: 'Gamma空间颜色' }],
        returnType: 'half3',
        category: 'Color'
    },
    {
        name: 'LinearToGammaSpace',
        signature: 'half3 LinearToGammaSpace(half3 linRGB)',
        description: '将颜色从线性空间转换到 Gamma 空间',
        parameters: [{ name: 'linRGB', type: 'half3', description: '线性空间颜色' }],
        returnType: 'half3',
        category: 'Color'
    },

    // 法线解码
    {
        name: 'UnpackNormal',
        signature: 'half3 UnpackNormal(half4 packednormal)',
        description: '从法线贴图解码法线',
        parameters: [{ name: 'packednormal', type: 'half4', description: '压缩的法线数据' }],
        returnType: 'half3',
        category: 'Normal'
    },
    {
        name: 'UnpackNormalWithScale',
        signature: 'half3 UnpackNormalWithScale(half4 packednormal, half scale)',
        description: '从法线贴图解码法线并应用缩放',
        parameters: [
            { name: 'packednormal', type: 'half4', description: '压缩的法线数据' },
            { name: 'scale', type: 'half', description: '法线强度缩放' }
        ],
        returnType: 'half3',
        category: 'Normal'
    },

    // HDR编码/解码
    {
        name: 'DecodeHDR',
        signature: 'half3 DecodeHDR(half4 data, half4 decodeInstructions)',
        description: '解码HDR颜色数据',
        parameters: [
            { name: 'data', type: 'half4', description: '编码的HDR数据' },
            { name: 'decodeInstructions', type: 'half4', description: '解码指令' }
        ],
        returnType: 'half3',
        category: 'Color'
    },
    {
        name: 'EncodeFloatRGBA',
        signature: 'float4 EncodeFloatRGBA(float v)',
        description: '将浮点数编码为RGBA',
        parameters: [{ name: 'v', type: 'float', description: '要编码的浮点数' }],
        returnType: 'float4',
        category: 'Color'
    },
    {
        name: 'DecodeFloatRGBA',
        signature: 'float DecodeFloatRGBA(float4 enc)',
        description: '从RGBA解码浮点数',
        parameters: [{ name: 'enc', type: 'float4', description: '编码的RGBA数据' }],
        returnType: 'float',
        category: 'Color'
    },
    {
        name: 'DecodeFloatRG',
        signature: 'float DecodeFloatRG(float2 enc)',
        description: '从RG解码浮点数',
        parameters: [{ name: 'enc', type: 'float2', description: '编码的RG数据' }],
        returnType: 'float',
        category: 'Color'
    },
    {
        name: 'EncodeFloatRG',
        signature: 'float2 EncodeFloatRG(float v)',
        description: '将浮点数编码为RG',
        parameters: [{ name: 'v', type: 'float', description: '要编码的浮点数' }],
        returnType: 'float2',
        category: 'Color'
    },
];

// ============================================================================
// Unity 常用宏定义
// ============================================================================

export interface UnityMacro {
    name: string;
    description: string;
    usage?: string;
    category: string;
}

export const unityBuiltinMacros: UnityMacro[] = [
    // 纹理采样宏
    { name: 'TRANSFORM_TEX', description: '应用纹理的缩放和偏移', usage: 'TRANSFORM_TEX(uv, _MainTex)', category: 'Texture' },
    { name: 'SAMPLE_TEXTURE2D', description: 'SRP兼容的2D纹理采样', usage: 'SAMPLE_TEXTURE2D(tex, sampler, uv)', category: 'Texture' },
    { name: 'SAMPLE_TEXTURE2D_LOD', description: 'SRP兼容的2D纹理LOD采样', usage: 'SAMPLE_TEXTURE2D_LOD(tex, sampler, uv, lod)', category: 'Texture' },
    { name: 'SAMPLE_TEXTURECUBE', description: 'SRP兼容的立方体纹理采样', usage: 'SAMPLE_TEXTURECUBE(tex, sampler, dir)', category: 'Texture' },
    { name: 'SAMPLE_TEXTURE3D', description: 'SRP兼容的3D纹理采样', usage: 'SAMPLE_TEXTURE3D(tex, sampler, uvw)', category: 'Texture' },
    
    // 纹理声明宏
    { name: 'UNITY_DECLARE_TEX2D', description: '声明2D纹理', usage: 'UNITY_DECLARE_TEX2D(_MainTex)', category: 'Texture' },
    { name: 'UNITY_DECLARE_TEX2D_NOSAMPLER', description: '声明不带采样器的2D纹理', usage: 'UNITY_DECLARE_TEX2D_NOSAMPLER(_MainTex)', category: 'Texture' },
    { name: 'UNITY_DECLARE_TEXCUBE', description: '声明立方体纹理', usage: 'UNITY_DECLARE_TEXCUBE(_CubeMap)', category: 'Texture' },
    { name: 'UNITY_SAMPLE_TEX2D', description: 'Unity兼容的2D纹理采样', usage: 'UNITY_SAMPLE_TEX2D(_MainTex, uv)', category: 'Texture' },
    { name: 'UNITY_SAMPLE_TEX2D_SAMPLER', description: '使用指定采样器的2D纹理采样', usage: 'UNITY_SAMPLE_TEX2D_SAMPLER(_MainTex, sampler_MainTex, uv)', category: 'Texture' },

    // 雾效宏
    { name: 'UNITY_FOG_COORDS', description: '声明雾效坐标', usage: 'UNITY_FOG_COORDS(1)', category: 'Fog' },
    { name: 'UNITY_TRANSFER_FOG', description: '传递雾效数据', usage: 'UNITY_TRANSFER_FOG(o, o.vertex)', category: 'Fog' },
    { name: 'UNITY_APPLY_FOG', description: '应用雾效', usage: 'UNITY_APPLY_FOG(i.fogCoord, col)', category: 'Fog' },

    // 阴影宏
    { name: 'SHADOW_COORDS', description: '声明阴影坐标', usage: 'SHADOW_COORDS(1)', category: 'Shadow' },
    { name: 'TRANSFER_SHADOW', description: '传递阴影数据', usage: 'TRANSFER_SHADOW(o)', category: 'Shadow' },
    { name: 'SHADOW_ATTENUATION', description: '获取阴影衰减', usage: 'SHADOW_ATTENUATION(i)', category: 'Shadow' },
    { name: 'UNITY_SHADOW_COORDS', description: '声明Unity阴影坐标', usage: 'UNITY_SHADOW_COORDS(1)', category: 'Shadow' },
    { name: 'UNITY_TRANSFER_SHADOW', description: '传递Unity阴影数据', usage: 'UNITY_TRANSFER_SHADOW(o, v.texcoord)', category: 'Shadow' },
    { name: 'UNITY_SHADOW_ATTENUATION', description: '获取Unity阴影衰减', usage: 'UNITY_SHADOW_ATTENUATION(i, worldPos)', category: 'Shadow' },
    { name: 'UNITY_LIGHT_ATTENUATION', description: '获取光照衰减（包含阴影）', usage: 'UNITY_LIGHT_ATTENUATION(atten, i, worldPos)', category: 'Shadow' },

    // 实例化宏
    { name: 'UNITY_VERTEX_INPUT_INSTANCE_ID', description: '声明顶点实例ID', usage: 'UNITY_VERTEX_INPUT_INSTANCE_ID', category: 'Instancing' },
    { name: 'UNITY_VERTEX_OUTPUT_STEREO', description: '声明立体渲染输出', usage: 'UNITY_VERTEX_OUTPUT_STEREO', category: 'Instancing' },
    { name: 'UNITY_SETUP_INSTANCE_ID', description: '设置实例ID', usage: 'UNITY_SETUP_INSTANCE_ID(v)', category: 'Instancing' },
    { name: 'UNITY_TRANSFER_INSTANCE_ID', description: '传递实例ID', usage: 'UNITY_TRANSFER_INSTANCE_ID(v, o)', category: 'Instancing' },
    { name: 'UNITY_SETUP_STEREO_EYE_INDEX_POST_VERTEX', description: '设置立体眼睛索引', usage: 'UNITY_SETUP_STEREO_EYE_INDEX_POST_VERTEX(i)', category: 'Instancing' },
    { name: 'UNITY_INITIALIZE_OUTPUT', description: '初始化输出结构', usage: 'UNITY_INITIALIZE_OUTPUT(v2f, o)', category: 'Instancing' },
    { name: 'UNITY_INITIALIZE_VERTEX_OUTPUT_STEREO', description: '初始化立体渲染输出', usage: 'UNITY_INITIALIZE_VERTEX_OUTPUT_STEREO(o)', category: 'Instancing' },
];

// ============================================================================
// ShaderLab 关键字定义
// ============================================================================

export interface ShaderLabKeyword {
    name: string;
    description: string;
    snippet?: string;
    category: string;
}

export const shaderLabKeywords: ShaderLabKeyword[] = [
    // 结构关键字
    { name: 'Shader', description: 'Shader 声明', snippet: 'Shader "${1:Custom/MyShader}" {\n\t$0\n}', category: 'Structure' },
    { name: 'Properties', description: '属性块', snippet: 'Properties {\n\t$0\n}', category: 'Structure' },
    { name: 'SubShader', description: '子着色器', snippet: 'SubShader {\n\tTags { ${1:"RenderType"="Opaque"} }\n\t$0\n}', category: 'Structure' },
    { name: 'Pass', description: '渲染通道', snippet: 'Pass {\n\t$0\n}', category: 'Structure' },
    { name: 'Category', description: '类别', snippet: 'Category {\n\t$0\n}', category: 'Structure' },
    { name: 'CGPROGRAM', description: 'CG程序块开始', snippet: 'CGPROGRAM\n$0\nENDCG', category: 'Structure' },
    { name: 'ENDCG', description: 'CG程序块结束', category: 'Structure' },
    { name: 'CGINCLUDE', description: 'CG包含块开始', snippet: 'CGINCLUDE\n$0\nENDCG', category: 'Structure' },
    { name: 'HLSLPROGRAM', description: 'HLSL程序块开始', snippet: 'HLSLPROGRAM\n$0\nENDHLSL', category: 'Structure' },
    { name: 'ENDHLSL', description: 'HLSL程序块结束', category: 'Structure' },
    { name: 'HLSLINCLUDE', description: 'HLSL包含块开始', snippet: 'HLSLINCLUDE\n$0\nENDHLSL', category: 'Structure' },
    
    // 渲染状态
    { name: 'Blend', description: '混合模式', snippet: 'Blend ${1:SrcAlpha} ${2:OneMinusSrcAlpha}', category: 'RenderState' },
    { name: 'BlendOp', description: '混合操作', snippet: 'BlendOp ${1:Add}', category: 'RenderState' },
    { name: 'Cull', description: '剔除模式', snippet: 'Cull ${1|Off,Front,Back|}', category: 'RenderState' },
    { name: 'ZWrite', description: '深度写入', snippet: 'ZWrite ${1|On,Off|}', category: 'RenderState' },
    { name: 'ZTest', description: '深度测试', snippet: 'ZTest ${1|LEqual,Less,Greater,GEqual,Equal,NotEqual,Always|}', category: 'RenderState' },
    { name: 'ColorMask', description: '颜色遮罩', snippet: 'ColorMask ${1|RGBA,RGB,A,0|}', category: 'RenderState' },
    { name: 'Offset', description: '深度偏移', snippet: 'Offset ${1:-1}, ${2:-1}', category: 'RenderState' },
    { name: 'Stencil', description: '模板测试', snippet: 'Stencil {\n\tRef ${1:1}\n\tComp ${2:Always}\n\tPass ${3:Replace}\n}', category: 'RenderState' },

    // Tags
    { name: 'Tags', description: '标签块', snippet: 'Tags { ${1:"RenderType"="${2:Opaque}"} }', category: 'Tags' },
    { name: 'LOD', description: '细节层次', snippet: 'LOD ${1:200}', category: 'Tags' },
    { name: 'FallBack', description: '回退着色器', snippet: 'FallBack "${1:Diffuse}"', category: 'Tags' },
    { name: 'CustomEditor', description: '自定义编辑器', snippet: 'CustomEditor "${1:MyShaderEditor}"', category: 'Tags' },
    { name: 'UsePass', description: '使用其他Shader的Pass', snippet: 'UsePass "${1:Legacy Shaders/VertexLit/SHADOWCASTER}"', category: 'Tags' },
    { name: 'GrabPass', description: '抓取屏幕内容', snippet: 'GrabPass { "${1:_GrabTexture}" }', category: 'Tags' },
];

// ============================================================================
// ShaderLab 属性类型
// ============================================================================

export interface ShaderLabPropertyType {
    name: string;
    description: string;
    defaultValue: string;
    example: string;
}

export const shaderLabPropertyTypes: ShaderLabPropertyType[] = [
    { name: '2D', description: '2D纹理', defaultValue: '"white" {}', example: '_MainTex ("Texture", 2D) = "white" {}' },
    { name: '3D', description: '3D纹理', defaultValue: '"" {}', example: '_Volume ("Volume", 3D) = "" {}' },
    { name: 'Cube', description: '立方体贴图', defaultValue: '"_Skybox" {}', example: '_Cube ("Cubemap", Cube) = "_Skybox" {}' },
    { name: 'CubeArray', description: '立方体贴图数组', defaultValue: '"" {}', example: '_CubeArray ("Cube Array", CubeArray) = "" {}' },
    { name: '2DArray', description: '2D纹理数组', defaultValue: '"" {}', example: '_TexArray ("Texture Array", 2DArray) = "" {}' },
    { name: 'Color', description: '颜色', defaultValue: '(1,1,1,1)', example: '_Color ("Color", Color) = (1,1,1,1)' },
    { name: 'Vector', description: '向量', defaultValue: '(0,0,0,0)', example: '_Vector ("Vector", Vector) = (0,0,0,0)' },
    { name: 'Float', description: '浮点数', defaultValue: '1.0', example: '_Float ("Float", Float) = 1.0' },
    { name: 'Int', description: '整数', defaultValue: '1', example: '_Int ("Int", Int) = 1' },
    { name: 'Integer', description: '整数（新版）', defaultValue: '1', example: '_Integer ("Integer", Integer) = 1' },
    { name: 'Range', description: '范围滑块', defaultValue: '0.5', example: '_Range ("Range", Range(0, 1)) = 0.5' },
];

// ============================================================================
// Pragma 指令
// ============================================================================

export interface PragmaDirective {
    name: string;
    description: string;
    example: string;
}

export const pragmaDirectives: PragmaDirective[] = [
    { name: '#pragma vertex', description: '指定顶点着色器函数', example: '#pragma vertex vert' },
    { name: '#pragma fragment', description: '指定片元着色器函数', example: '#pragma fragment frag' },
    { name: '#pragma geometry', description: '指定几何着色器函数', example: '#pragma geometry geom' },
    { name: '#pragma hull', description: '指定 Hull 着色器函数', example: '#pragma hull hull' },
    { name: '#pragma domain', description: '指定 Domain 着色器函数', example: '#pragma domain domain' },
    { name: '#pragma surface', description: '指定表面着色器函数', example: '#pragma surface surf Standard fullforwardshadows' },
    { name: '#pragma target', description: '指定着色器目标级别', example: '#pragma target 3.0' },
    { name: '#pragma multi_compile', description: '多编译变体（全局）', example: '#pragma multi_compile _ _KEYWORD_ON' },
    { name: '#pragma multi_compile_local', description: '多编译变体（本地）', example: '#pragma multi_compile_local _ _KEYWORD_ON' },
    { name: '#pragma shader_feature', description: '着色器特性（全局）', example: '#pragma shader_feature _NORMALMAP' },
    { name: '#pragma shader_feature_local', description: '着色器特性（本地）', example: '#pragma shader_feature_local _NORMALMAP' },
    { name: '#pragma multi_compile_fog', description: '雾效变体', example: '#pragma multi_compile_fog' },
    { name: '#pragma multi_compile_instancing', description: 'GPU实例化变体', example: '#pragma multi_compile_instancing' },
    { name: '#pragma multi_compile_shadowcaster', description: '阴影投射变体', example: '#pragma multi_compile_shadowcaster' },
    { name: '#pragma multi_compile_fwdbase', description: '前向基础光照变体', example: '#pragma multi_compile_fwdbase' },
    { name: '#pragma multi_compile_fwdadd', description: '前向附加光照变体', example: '#pragma multi_compile_fwdadd' },
    { name: '#pragma multi_compile_fwdadd_fullshadows', description: '前向附加光照变体（带阴影）', example: '#pragma multi_compile_fwdadd_fullshadows' },
    { name: '#pragma only_renderers', description: '限制渲染器', example: '#pragma only_renderers d3d11 vulkan metal' },
    { name: '#pragma exclude_renderers', description: '排除渲染器', example: '#pragma exclude_renderers gles' },
    { name: '#pragma enable_d3d11_debug_symbols', description: '启用D3D11调试符号', example: '#pragma enable_d3d11_debug_symbols' },
    { name: '#pragma instancing_options', description: 'GPU实例化选项', example: '#pragma instancing_options assumeuniformscaling' },
];

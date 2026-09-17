/**
 * Vendored URP HLSL tables from
 * https://github.com/yyqrc/vscode-unitylabshader (MIT).
 *
 * URP (Universal Render Pipeline) 内置定义
 * 包含 URP 特有的变量、函数、宏定义用于代码补全和悬停提示
 */

// ============================================================================
// URP 内置变量定义
// ============================================================================

export interface URPVariable {
    name: string;
    type: string;
    description: string;
    category: string;
}

export const urpBuiltinVariables: URPVariable[] = [
    // 主光源
    { name: '_MainLightPosition', type: 'half4', description: '主光源方向（xyz）和类型标志（w）', category: 'MainLight' },
    { name: '_MainLightColor', type: 'half4', description: '主光源颜色', category: 'MainLight' },
    { name: '_MainLightOcclusionProbes', type: 'half4', description: '主光源遮挡探针', category: 'MainLight' },
    { name: '_MainLightLayerMask', type: 'uint', description: '主光源渲染层掩码', category: 'MainLight' },

    // 附加光源
    { name: '_AdditionalLightsCount', type: 'half4', description: '附加光源数量和参数', category: 'AdditionalLights' },
    { name: '_AdditionalLightsPosition', type: 'float4[]', description: '附加光源位置数组', category: 'AdditionalLights' },
    { name: '_AdditionalLightsColor', type: 'half4[]', description: '附加光源颜色数组', category: 'AdditionalLights' },
    { name: '_AdditionalLightsAttenuation', type: 'half4[]', description: '附加光源衰减数组', category: 'AdditionalLights' },
    { name: '_AdditionalLightsSpotDir', type: 'half4[]', description: '附加聚光灯方向数组', category: 'AdditionalLights' },
    { name: '_AdditionalLightsOcclusionProbes', type: 'half4[]', description: '附加光源遮挡探针数组', category: 'AdditionalLights' },
    { name: '_AdditionalLightsLayerMasks', type: 'float[]', description: '附加光源层掩码数组', category: 'AdditionalLights' },

    // 阴影
    { name: '_MainLightWorldToShadow', type: 'float4x4[5]', description: '主光源世界到阴影空间变换矩阵', category: 'Shadow' },
    { name: '_CascadeShadowSplitSpheres0', type: 'float4', description: '级联阴影分割球体0', category: 'Shadow' },
    { name: '_CascadeShadowSplitSpheres1', type: 'float4', description: '级联阴影分割球体1', category: 'Shadow' },
    { name: '_CascadeShadowSplitSpheres2', type: 'float4', description: '级联阴影分割球体2', category: 'Shadow' },
    { name: '_CascadeShadowSplitSpheres3', type: 'float4', description: '级联阴影分割球体3', category: 'Shadow' },
    { name: '_MainLightShadowParams', type: 'float4', description: '主光源阴影参数', category: 'Shadow' },

    // 全局光照
    { name: 'unity_ProbesOcclusion', type: 'half4', description: '探针遮挡数据', category: 'GI' },
    { name: 'unity_SpecCube0', type: 'TextureCube', description: '反射探针立方体贴图', category: 'GI' },
    { name: 'unity_SpecCube0_HDR', type: 'half4', description: '反射探针HDR解码参数', category: 'GI' },

    // 相机和渲染目标
    { name: '_ScaledScreenParams', type: 'float4', description: '缩放后的屏幕参数', category: 'Camera' },
    { name: '_CameraDepthTexture', type: 'Texture2D', description: '相机深度纹理', category: 'Camera' },
    { name: '_CameraOpaqueTexture', type: 'Texture2D', description: '相机不透明纹理', category: 'Camera' },
    { name: '_CameraColorTexture', type: 'Texture2D', description: '相机颜色纹理', category: 'Camera' },
];

// ============================================================================
// URP 内置函数定义
// ============================================================================

export interface URPFunction {
    name: string;
    signature: string;
    description: string;
    parameters: { name: string; type: string; description: string }[];
    returnType: string;
    category: string;
}

export const urpBuiltinFunctions: URPFunction[] = [
    // 空间变换函数
    {
        name: 'TransformObjectToHClip',
        signature: 'float4 TransformObjectToHClip(float3 positionOS)',
        description: '将顶点从模型空间变换到齐次裁剪空间',
        parameters: [{ name: 'positionOS', type: 'float3', description: '模型空间顶点位置' }],
        returnType: 'float4',
        category: 'Transform'
    },
    {
        name: 'TransformObjectToWorld',
        signature: 'float3 TransformObjectToWorld(float3 positionOS)',
        description: '将顶点从模型空间变换到世界空间',
        parameters: [{ name: 'positionOS', type: 'float3', description: '模型空间顶点位置' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'TransformWorldToHClip',
        signature: 'float4 TransformWorldToHClip(float3 positionWS)',
        description: '将顶点从世界空间变换到齐次裁剪空间',
        parameters: [{ name: 'positionWS', type: 'float3', description: '世界空间顶点位置' }],
        returnType: 'float4',
        category: 'Transform'
    },
    {
        name: 'TransformWorldToView',
        signature: 'float3 TransformWorldToView(float3 positionWS)',
        description: '将顶点从世界空间变换到视图空间',
        parameters: [{ name: 'positionWS', type: 'float3', description: '世界空间顶点位置' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'TransformObjectToWorldNormal',
        signature: 'float3 TransformObjectToWorldNormal(float3 normalOS)',
        description: '将法线从模型空间变换到世界空间',
        parameters: [{ name: 'normalOS', type: 'float3', description: '模型空间法线' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'TransformObjectToWorldDir',
        signature: 'float3 TransformObjectToWorldDir(float3 dirOS)',
        description: '将方向向量从模型空间变换到世界空间',
        parameters: [{ name: 'dirOS', type: 'float3', description: '模型空间方向' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'TransformWorldToObjectDir',
        signature: 'float3 TransformWorldToObjectDir(float3 dirWS)',
        description: '将方向向量从世界空间变换到模型空间',
        parameters: [{ name: 'dirWS', type: 'float3', description: '世界空间方向' }],
        returnType: 'float3',
        category: 'Transform'
    },
    {
        name: 'TransformTangentToWorld',
        signature: 'real3 TransformTangentToWorld(real3 dirTS, real3x3 tangentToWorld)',
        description: '将方向从切线空间变换到世界空间',
        parameters: [
            { name: 'dirTS', type: 'real3', description: '切线空间方向' },
            { name: 'tangentToWorld', type: 'real3x3', description: '切线到世界空间变换矩阵' }
        ],
        returnType: 'real3',
        category: 'Transform'
    },
    {
        name: 'GetVertexPositionInputs',
        signature: 'VertexPositionInputs GetVertexPositionInputs(float3 positionOS)',
        description: '获取顶点位置输入结构体（包含各空间位置）',
        parameters: [{ name: 'positionOS', type: 'float3', description: '模型空间顶点位置' }],
        returnType: 'VertexPositionInputs',
        category: 'Transform'
    },
    {
        name: 'GetVertexNormalInputs',
        signature: 'VertexNormalInputs GetVertexNormalInputs(float3 normalOS, float4 tangentOS)',
        description: '获取顶点法线输入结构体',
        parameters: [
            { name: 'normalOS', type: 'float3', description: '模型空间法线' },
            { name: 'tangentOS', type: 'float4', description: '模型空间切线' }
        ],
        returnType: 'VertexNormalInputs',
        category: 'Transform'
    },

    // 光照函数
    {
        name: 'GetMainLight',
        signature: 'Light GetMainLight()',
        description: '获取主光源信息',
        parameters: [],
        returnType: 'Light',
        category: 'Lighting'
    },
    {
        name: 'GetMainLight',
        signature: 'Light GetMainLight(float4 shadowCoord)',
        description: '获取主光源信息（带阴影）',
        parameters: [{ name: 'shadowCoord', type: 'float4', description: '阴影坐标' }],
        returnType: 'Light',
        category: 'Lighting'
    },
    {
        name: 'GetAdditionalLightsCount',
        signature: 'int GetAdditionalLightsCount()',
        description: '获取附加光源数量',
        parameters: [],
        returnType: 'int',
        category: 'Lighting'
    },
    {
        name: 'GetAdditionalLight',
        signature: 'Light GetAdditionalLight(uint i, float3 positionWS)',
        description: '获取指定索引的附加光源',
        parameters: [
            { name: 'i', type: 'uint', description: '光源索引' },
            { name: 'positionWS', type: 'float3', description: '世界空间位置' }
        ],
        returnType: 'Light',
        category: 'Lighting'
    },
    {
        name: 'LightingLambert',
        signature: 'half3 LightingLambert(half3 lightColor, half3 lightDir, half3 normal)',
        description: 'Lambert 漫反射光照计算',
        parameters: [
            { name: 'lightColor', type: 'half3', description: '光源颜色' },
            { name: 'lightDir', type: 'half3', description: '光源方向' },
            { name: 'normal', type: 'half3', description: '表面法线' }
        ],
        returnType: 'half3',
        category: 'Lighting'
    },
    {
        name: 'LightingSpecular',
        signature: 'half3 LightingSpecular(half3 lightColor, half3 lightDir, half3 normal, half3 viewDir, half4 specular, half smoothness)',
        description: 'Blinn-Phong 高光计算',
        parameters: [
            { name: 'lightColor', type: 'half3', description: '光源颜色' },
            { name: 'lightDir', type: 'half3', description: '光源方向' },
            { name: 'normal', type: 'half3', description: '表面法线' },
            { name: 'viewDir', type: 'half3', description: '视线方向' },
            { name: 'specular', type: 'half4', description: '高光颜色' },
            { name: 'smoothness', type: 'half', description: '光滑度' }
        ],
        returnType: 'half3',
        category: 'Lighting'
    },
    {
        name: 'UniversalFragmentPBR',
        signature: 'half4 UniversalFragmentPBR(InputData inputData, SurfaceData surfaceData)',
        description: 'URP PBR 光照计算',
        parameters: [
            { name: 'inputData', type: 'InputData', description: '输入数据结构' },
            { name: 'surfaceData', type: 'SurfaceData', description: '表面数据结构' }
        ],
        returnType: 'half4',
        category: 'Lighting'
    },
    {
        name: 'UniversalFragmentBlinnPhong',
        signature: 'half4 UniversalFragmentBlinnPhong(InputData inputData, SurfaceData surfaceData)',
        description: 'URP Blinn-Phong 光照计算',
        parameters: [
            { name: 'inputData', type: 'InputData', description: '输入数据结构' },
            { name: 'surfaceData', type: 'SurfaceData', description: '表面数据结构' }
        ],
        returnType: 'half4',
        category: 'Lighting'
    },

    // 阴影函数
    {
        name: 'GetShadowCoord',
        signature: 'float4 GetShadowCoord(VertexPositionInputs vertexInput)',
        description: '获取阴影坐标',
        parameters: [{ name: 'vertexInput', type: 'VertexPositionInputs', description: '顶点位置输入' }],
        returnType: 'float4',
        category: 'Shadow'
    },
    {
        name: 'TransformWorldToShadowCoord',
        signature: 'float4 TransformWorldToShadowCoord(float3 positionWS)',
        description: '将世界空间位置变换到阴影坐标',
        parameters: [{ name: 'positionWS', type: 'float3', description: '世界空间位置' }],
        returnType: 'float4',
        category: 'Shadow'
    },
    {
        name: 'MainLightRealtimeShadow',
        signature: 'half MainLightRealtimeShadow(float4 shadowCoord)',
        description: '计算主光源实时阴影衰减',
        parameters: [{ name: 'shadowCoord', type: 'float4', description: '阴影坐标' }],
        returnType: 'half',
        category: 'Shadow'
    },
    {
        name: 'AdditionalLightRealtimeShadow',
        signature: 'half AdditionalLightRealtimeShadow(int lightIndex, float3 positionWS)',
        description: '计算附加光源实时阴影衰减',
        parameters: [
            { name: 'lightIndex', type: 'int', description: '光源索引' },
            { name: 'positionWS', type: 'float3', description: '世界空间位置' }
        ],
        returnType: 'half',
        category: 'Shadow'
    },

    // GI 函数
    {
        name: 'SampleSH',
        signature: 'half3 SampleSH(half3 normalWS)',
        description: '采样球谐光照',
        parameters: [{ name: 'normalWS', type: 'half3', description: '世界空间法线' }],
        returnType: 'half3',
        category: 'GI'
    },
    {
        name: 'GlossyEnvironmentReflection',
        signature: 'half3 GlossyEnvironmentReflection(half3 reflectVector, half perceptualRoughness, half occlusion)',
        description: '采样光泽环境反射',
        parameters: [
            { name: 'reflectVector', type: 'half3', description: '反射向量' },
            { name: 'perceptualRoughness', type: 'half', description: '感知粗糙度' },
            { name: 'occlusion', type: 'half', description: '遮挡值' }
        ],
        returnType: 'half3',
        category: 'GI'
    },
    {
        name: 'SampleLightmap',
        signature: 'half3 SampleLightmap(float2 lightmapUV, half3 normalWS)',
        description: '采样光照贴图',
        parameters: [
            { name: 'lightmapUV', type: 'float2', description: '光照贴图UV' },
            { name: 'normalWS', type: 'half3', description: '世界空间法线' }
        ],
        returnType: 'half3',
        category: 'GI'
    },

    // 深度函数
    {
        name: 'LinearEyeDepth',
        signature: 'real LinearEyeDepth(real depth, float4 zBufferParam)',
        description: '将深度缓冲值转换为线性视图空间深度',
        parameters: [
            { name: 'depth', type: 'real', description: '深度缓冲值' },
            { name: 'zBufferParam', type: 'float4', description: 'Z缓冲参数' }
        ],
        returnType: 'real',
        category: 'Depth'
    },
    {
        name: 'Linear01Depth',
        signature: 'real Linear01Depth(real depth, float4 zBufferParam)',
        description: '将深度缓冲值转换为0-1范围的线性深度',
        parameters: [
            { name: 'depth', type: 'real', description: '深度缓冲值' },
            { name: 'zBufferParam', type: 'float4', description: 'Z缓冲参数' }
        ],
        returnType: 'real',
        category: 'Depth'
    },

    // 法线函数
    {
        name: 'UnpackNormal',
        signature: 'real3 UnpackNormal(real4 packedNormal)',
        description: '从法线贴图解压法线',
        parameters: [{ name: 'packedNormal', type: 'real4', description: '压缩的法线数据' }],
        returnType: 'real3',
        category: 'Normal'
    },
    {
        name: 'UnpackNormalScale',
        signature: 'real3 UnpackNormalScale(real4 packedNormal, real scale)',
        description: '从法线贴图解压法线并应用缩放',
        parameters: [
            { name: 'packedNormal', type: 'real4', description: '压缩的法线数据' },
            { name: 'scale', type: 'real', description: '法线强度缩放' }
        ],
        returnType: 'real3',
        category: 'Normal'
    },
    {
        name: 'NormalizeNormalPerPixel',
        signature: 'half3 NormalizeNormalPerPixel(half3 normalWS)',
        description: '逐像素归一化法线',
        parameters: [{ name: 'normalWS', type: 'half3', description: '世界空间法线' }],
        returnType: 'half3',
        category: 'Normal'
    },

    // 雾效函数
    {
        name: 'ComputeFogFactor',
        signature: 'real ComputeFogFactor(float z)',
        description: '计算雾效因子',
        parameters: [{ name: 'z', type: 'float', description: '裁剪空间Z坐标' }],
        returnType: 'real',
        category: 'Fog'
    },
    {
        name: 'MixFog',
        signature: 'half3 MixFog(half3 fragColor, half fogFactor)',
        description: '混合雾效颜色',
        parameters: [
            { name: 'fragColor', type: 'half3', description: '片元颜色' },
            { name: 'fogFactor', type: 'half', description: '雾效因子' }
        ],
        returnType: 'half3',
        category: 'Fog'
    },
];

// ============================================================================
// URP 常用宏定义
// ============================================================================

export interface URPMacro {
    name: string;
    description: string;
    usage?: string;
    category: string;
}

export const urpBuiltinMacros: URPMacro[] = [
    // 关键字宏
    { name: '_MAIN_LIGHT_SHADOWS', description: '启用主光源阴影', category: 'Keyword' },
    { name: '_MAIN_LIGHT_SHADOWS_CASCADE', description: '启用级联阴影', category: 'Keyword' },
    { name: '_MAIN_LIGHT_SHADOWS_SCREEN', description: '启用屏幕空间阴影', category: 'Keyword' },
    { name: '_ADDITIONAL_LIGHTS', description: '启用附加光源', category: 'Keyword' },
    { name: '_ADDITIONAL_LIGHTS_VERTEX', description: '启用顶点附加光照', category: 'Keyword' },
    { name: '_ADDITIONAL_LIGHT_SHADOWS', description: '启用附加光源阴影', category: 'Keyword' },
    { name: '_SHADOWS_SOFT', description: '启用软阴影', category: 'Keyword' },
    { name: '_MIXED_LIGHTING_SUBTRACTIVE', description: '混合照明减法模式', category: 'Keyword' },
    { name: '_SCREEN_SPACE_OCCLUSION', description: '启用屏幕空间遮挡', category: 'Keyword' },
    { name: '_NORMALMAP', description: '启用法线贴图', category: 'Keyword' },
    { name: '_EMISSION', description: '启用自发光', category: 'Keyword' },
    { name: '_SPECULAR_SETUP', description: '使用高光工作流', category: 'Keyword' },

    // 纹理宏
    { name: 'TEXTURE2D', description: '声明2D纹理', usage: 'TEXTURE2D(_MainTex)', category: 'Texture' },
    { name: 'TEXTURECUBE', description: '声明立方体纹理', usage: 'TEXTURECUBE(_CubeMap)', category: 'Texture' },
    { name: 'TEXTURE3D', description: '声明3D纹理', usage: 'TEXTURE3D(_Volume)', category: 'Texture' },
    { name: 'TEXTURE2D_ARRAY', description: '声明2D纹理数组', usage: 'TEXTURE2D_ARRAY(_TexArray)', category: 'Texture' },
    { name: 'SAMPLER', description: '声明采样器', usage: 'SAMPLER(sampler_MainTex)', category: 'Texture' },
    { name: 'SAMPLE_TEXTURE2D', description: '采样2D纹理', usage: 'SAMPLE_TEXTURE2D(_MainTex, sampler_MainTex, uv)', category: 'Texture' },
    { name: 'SAMPLE_TEXTURE2D_LOD', description: '采样2D纹理指定LOD', usage: 'SAMPLE_TEXTURE2D_LOD(_MainTex, sampler_MainTex, uv, lod)', category: 'Texture' },
    { name: 'SAMPLE_TEXTURECUBE', description: '采样立方体纹理', usage: 'SAMPLE_TEXTURECUBE(_CubeMap, sampler_CubeMap, dir)', category: 'Texture' },
    { name: 'SAMPLE_TEXTURECUBE_LOD', description: '采样立方体纹理指定LOD', usage: 'SAMPLE_TEXTURECUBE_LOD(_CubeMap, sampler_CubeMap, dir, lod)', category: 'Texture' },

    // CBuffer 宏
    { name: 'CBUFFER_START', description: '开始常量缓冲区', usage: 'CBUFFER_START(UnityPerMaterial)', category: 'CBuffer' },
    { name: 'CBUFFER_END', description: '结束常量缓冲区', usage: 'CBUFFER_END', category: 'CBuffer' },

    // 变换宏
    { name: 'TRANSFORM_TEX', description: '应用纹理缩放偏移', usage: 'TRANSFORM_TEX(uv, _MainTex)', category: 'Transform' },

    // 实例化宏
    { name: 'UNITY_SETUP_INSTANCE_ID', description: '设置实例ID', usage: 'UNITY_SETUP_INSTANCE_ID(input)', category: 'Instancing' },
    { name: 'UNITY_TRANSFER_INSTANCE_ID', description: '传递实例ID', usage: 'UNITY_TRANSFER_INSTANCE_ID(input, output)', category: 'Instancing' },
    { name: 'UNITY_VERTEX_INPUT_INSTANCE_ID', description: '声明顶点输入实例ID', category: 'Instancing' },

    // 立体渲染宏
    { name: 'UNITY_INITIALIZE_VERTEX_OUTPUT_STEREO', description: '初始化立体渲染输出', usage: 'UNITY_INITIALIZE_VERTEX_OUTPUT_STEREO(output)', category: 'Stereo' },
    { name: 'UNITY_SETUP_STEREO_EYE_INDEX_POST_VERTEX', description: '设置立体眼睛索引', usage: 'UNITY_SETUP_STEREO_EYE_INDEX_POST_VERTEX(input)', category: 'Stereo' },
];

// ============================================================================
// URP Include 路径
// ============================================================================

export const urpIncludePaths: string[] = [
    'Packages/com.unity.render-pipelines.universal/ShaderLibrary/Core.hlsl',
    'Packages/com.unity.render-pipelines.universal/ShaderLibrary/Lighting.hlsl',
    'Packages/com.unity.render-pipelines.universal/ShaderLibrary/Shadows.hlsl',
    'Packages/com.unity.render-pipelines.universal/ShaderLibrary/Input.hlsl',
    'Packages/com.unity.render-pipelines.universal/ShaderLibrary/SurfaceInput.hlsl',
    'Packages/com.unity.render-pipelines.universal/ShaderLibrary/DeclareDepthTexture.hlsl',
    'Packages/com.unity.render-pipelines.universal/ShaderLibrary/DeclareOpaqueTexture.hlsl',
    'Packages/com.unity.render-pipelines.core/ShaderLibrary/Common.hlsl',
    'Packages/com.unity.render-pipelines.core/ShaderLibrary/Color.hlsl',
    'Packages/com.unity.render-pipelines.core/ShaderLibrary/Packing.hlsl',
];


// Declaration of all textures

layout(set=0, binding = 1) uniform sampler2D aces_reach_m_table_0Sampler; 
layout(set=0, binding = 2) uniform sampler2D aces_gamut_cusp_table_0Sampler; 

// Declaration of all helper methods

float aces_reach_m_table_0_sample(float h)
{
  float i_base = floor(h);
  float i_lo = i_base + float(1);
  float i_hi = i_lo + 1.0;
  float lo = texture(aces_reach_m_table_0Sampler, vec2((i_lo + 0.5) / float (363), 0.0)).r;
  float hi = texture(aces_reach_m_table_0Sampler, vec2((i_hi + 0.5) / float (363), 0.5)).r;
  float t = h - i_base;
  return mix(lo, hi, t);
}
float aces_tonescale_fwd0(float J)
{
  float A = 0.0323680267 * pow(abs(J) * 0.00999999978, 0.879464149);
  float Y = pow(( 27.1299992 * A) / (1.0f - A), 2.3809523809523809);
  float f = 20.4187794 * pow(Y / (Y + 8.1788140800216524), 1.14999998);
  float Y_ts = max(0.0, f * f / (f + 0.0399999991));
  float F_L_Y = pow(0.79370057210326195 * Y_ts, 0.42);
  float J_ts = 100. * pow((F_L_Y / ( 27.1299992 + F_L_Y)) * 30.8946857, 1.13705599);
  return sign(J) * J_ts;
}
float aces_toe_fwd0(float x, float limit, float k1_in, float k2_in)
{
  float k2 = max(k2_in, 0.001);
  float k1 = sqrt(k1_in * k1_in + k2 * k2);
  float k3 = (limit + k1) / (limit + k2);
  return (x > limit) ? x : 0.5 * (k3 * x - k1 + sqrt((k3 * x - k1) * (k3 * x - k1) + 4.0 * k2 * k3 * x));
}
const float aces_gamut_cusp_table_0_hues_array[363] = float[363](-0.99697876, 0., 0.989828587, 1.97965717, 2.96948576, 3.95931435, 4.94914293, 5.93897152, 6.92880011, 7.91862869, 8.9084568, 9.89828587, 10.8881149, 11.877943, 12.8677711, 13.8576002, 14.8474293, 15.8372574, 16.8270855, 17.8169136, 18.8067436, 19.7965717, 20.7863998, 21.7762299, 22.766058, 23.7558861, 25.0888977, 26.4219112, 27.4293823, 28.4368553, 29.4443264, 30.4517975, 31.4592686, 32.4667397, 33.4742126, 34.4816818, 35.4891548, 36.4966278, 37.504097, 38.51157, 39.519043, 40.5265121, 41.5339851, 42.5414581, 43.5489273, 44.5564003, 45.5638695, 46.5713425, 47.5788155, 48.5862846, 49.5937576, 50.6012268, 51.6086998, 52.6161728, 53.623642, 54.631115, 55.638588, 56.6460571, 57.6535301, 58.6610031, 59.6684723, 60.6759453, 61.6834145, 62.6908875, 63.6983604, 64.7058334, 65.7133026, 66.7207718, 67.7282486, 68.7357178, 69.743187, 70.7506561, 71.7581329, 72.7656021, 73.7730713, 74.7805481, 75.7880173, 76.7954865, 77.8029633, 78.8104324, 79.8179016, 80.8253784, 81.8328476, 82.8403168, 83.8477936, 84.8552628, 85.8627319, 86.8702087, 87.8776779, 88.8851471, 89.8926163, 90.9000931, 91.9075623, 92.9150314, 93.9225082, 94.9299774, 95.9374466, 96.9449234, 97.9523926, 98.9598618, 99.9673386, 100.974808, 101.982277, 102.989746, 103.997223, 105.004692, 106.012169, 106.431808, 107.425423, 108.419037, 109.412651, 110.406265, 111.399879, 112.393494, 113.387108, 114.380722, 115.374336, 116.36795, 117.361557, 118.355171, 119.348785, 120.3424, 121.336014, 122.329628, 123.323242, 124.316856, 125.310471, 126.304085, 127.297699, 128.291306, 129.284927, 130.278534, 131.272156, 132.265762, 133.259384, 134.252991, 135.246613, 136.240219, 137.233841, 138.227448, 139.221069, 140.214676, 141.208298, 142.201904, 143.195526, 144.189133, 145.182755, 146.176361, 146.846085, 147.515808, 148.550781, 149.585754, 150.620728, 151.655701, 152.690689, 153.725662, 154.760635, 155.795609, 156.830582, 157.865555, 158.900528, 159.935501, 160.97049, 162.005463, 163.040436, 164.075409, 165.110382, 166.145355, 167.180328, 168.215302, 169.250275, 170.285263, 171.320236, 172.355209, 173.390182, 174.425156, 175.460129, 176.495102, 177.530075, 178.565063, 179.600037, 180.63501, 181.669983, 182.704956, 183.739929, 184.774902, 185.809875, 186.844849, 187.879837, 188.91481, 189.949783, 190.984756, 192.01973, 192.127151, 193.128204, 194.129257, 195.13031, 196.131363, 197.132416, 198.133453, 199.134506, 200.135559, 201.136612, 202.137665, 203.138718, 204.139771, 205.140823, 206.141876, 207.142929, 208.143982, 209.14502, 210.146072, 211.147125, 212.148178, 213.149231, 214.150284, 215.151337, 216.15239, 217.153442, 218.154495, 219.155533, 220.156586, 221.157639, 222.158691, 223.159744, 224.160797, 225.16185, 226.162903, 227.163956, 228.165009, 229.166061, 230.167099, 231.168152, 232.169205, 233.170258, 234.17131, 235.172363, 236.173416, 237.174469, 238.175522, 239.176575, 240.177612, 241.178665, 242.179718, 243.180771, 244.181824, 245.182877, 246.183929, 247.184982, 248.186035, 249.187088, 250.188141, 251.189178, 252.190231, 253.191284, 254.192337, 255.19339, 256.194458, 257.195496, 258.196533, 259.197601, 260.198639, 261.199707, 262.200745, 263.201813, 264.20285, 265.203918, 266.204956, 267.016174, 267.827393, 268.638611, 269.653442, 270.668274, 271.683105, 272.697937, 273.712769, 274.72757, 275.742401, 276.757233, 277.772064, 278.786896, 279.801727, 280.816559, 281.83139, 282.846222, 283.861053, 284.875854, 285.890686, 286.905518, 287.920349, 288.935181, 289.950012, 290.964844, 291.979675, 292.994507, 294.009338, 295.024139, 296.038971, 297.053802, 298.068634, 299.083466, 300.098297, 301.113129, 302.12796, 303.142792, 304.157623, 305.172424, 306.187256, 307.202087, 308.216919, 309.23175, 310.246582, 311.261414, 312.276245, 313.291077, 314.305908, 315.32074, 316.335541, 317.350372, 318.365204, 319.380035, 320.394867, 321.409698, 322.42453, 323.439362, 324.454193, 325.469025, 326.483826, 327.498657, 328.513489, 329.52832, 330.543152, 331.557983, 332.572815, 333.587646, 334.602478, 335.61731, 336.632111, 337.646942, 338.661774, 339.676605, 340.691437, 341.706268, 342.054321, 343.0513, 344.048279, 345.045258, 346.042236, 347.039246, 348.036224, 349.033203, 350.030182, 351.027161, 352.024139, 353.021118, 354.018097, 355.015076, 356.012085, 357.009064, 358.006042, 359.003021, 360., 360.989838);
vec3 aces_gamut_cusp_table_0_sample(float h)
{
  int i = int(h) + 1;
  int i_lo = int(max(float(0), float(i + -1)));
  int i_hi = int(min(float(361), float(i + 2)));
  while (i_lo + 1 < i_hi)
  {
    float hcur = aces_gamut_cusp_table_0_hues_array[i];
    if (h > hcur)
    {
      i_lo = i;
    }
    else
    {
      i_hi = i;
    }
    i = (i_lo + i_hi) / 2;
  }
  vec3 lo = texture(aces_gamut_cusp_table_0Sampler, vec2((float(i_hi) - 1.0 + 0.5) / float(363), 0.5)).rgb;
  vec3 hi = texture(aces_gamut_cusp_table_0Sampler, vec2((float(i_hi) + 0.5) / float(363), 0.5)).rgb;
  float t = (h - aces_gamut_cusp_table_0_hues_array[i_hi - 1]) / (aces_gamut_cusp_table_0_hues_array[i_hi] - aces_gamut_cusp_table_0_hues_array[i_hi - 1]);
  return mix(lo, hi, t);
}
float aces_get_focus_gain0(float J, float cuspJ)
{
  float thr = mix(cuspJ, 382.487122, 0.300000);
  if (J > thr)
  {
    float gain = ( 382.487122 - thr) / max(0.0001, 382.487122 - J);
    gain = log(gain)/log(10.0);
    return gain * gain + 1.0;
  }
  else
  {
    return 1.0;
  }
}
float aces_solve_J_intersect0(float J, float M, float focusJ, float slope_gain)
{
  float M_scaled = M / slope_gain;
  float a = M_scaled / focusJ;
  if (J < focusJ)
  {
    float b = 1.0 - M_scaled;
    float c = -J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b + root);
  }
  else
  {
    float b = - (1.0 + M_scaled + 382.487122 * a);
    float c = 382.487122 * M_scaled + J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b - root);
  }
}
float aces_find_gamut_boundary_intersection0(vec2 JM_cusp, float gamma_top_inv, float gamma_bottom_inv, float J_intersect_source, float J_intersect_cusp, float slope)
{
  float M_boundary_lower = J_intersect_cusp * pow(J_intersect_source / J_intersect_cusp, gamma_bottom_inv) / (JM_cusp.r / JM_cusp.g - slope);
  float M_boundary_upper = JM_cusp.g * (382.487122 - J_intersect_cusp) * pow((382.487122 - J_intersect_source) / (382.487122 - J_intersect_cusp), gamma_top_inv) / (slope * JM_cusp.g + 382.487122 - JM_cusp.r);
  float smin = 0.0;
  {
    float a = M_boundary_lower;
    float b = M_boundary_upper;
    float s = 0.119999997 * JM_cusp.g;
    float h = max(s - abs(a - b), 0.0) / s;
    smin = min(a, b) - h * h * h * s * 0.16666666666666666;
  }
  return smin;
}
float aces_remap_M_fwd0(float M, float gamut_boundary_M, float reach_boundary_M)
{
  float boundary_ratio = gamut_boundary_M / reach_boundary_M;
  float proportion = max(boundary_ratio, 0.75);
  float threshold = proportion * gamut_boundary_M;
  if (proportion >= 1.0f || M <= threshold)
  {
    return M;
  }
  float m_offset = M - threshold;
  float gamut_offset = gamut_boundary_M - threshold;
  float reach_offset = reach_boundary_M - threshold;
  float scale = reach_offset / ((reach_offset / gamut_offset) - 1.0f);
  float nd = m_offset / scale;
  return threshold + scale * nd / (1.0f + nd);
}
vec3 aces_gamut_compress0(vec3 JMh, float Jx, vec3 JMGcusp, float reachMaxM)
{
  float J = JMh.r;
  float M = JMh.g;
  float h = JMh.b;
  if (M <= 0.0 || J > 382.487122)
  {
    return vec3(J, 0.0, h);
  }
  else
  {
    vec2 JMcusp = JMGcusp.rg;
    float focusJ = mix(JMcusp.r, 42.607346, min(1.0, 1.300000 - (JMcusp.r / 382.487122)));
    float slope_gain = 1692.00208 * aces_get_focus_gain0(Jx, JMcusp.r);
    float J_intersect_source = aces_solve_J_intersect0(JMh.r, JMh.g, focusJ, slope_gain);
    float gamut_slope = (J_intersect_source < focusJ) ? J_intersect_source : (382.487122 - J_intersect_source);
    gamut_slope = gamut_slope * (J_intersect_source - focusJ) / (focusJ * slope_gain);
    float gamma_top_inv = JMGcusp.b;
    float gamma_bottom_inv = 0.812300146;
    float J_intersect_cusp = aces_solve_J_intersect0(JMcusp.r, JMcusp.g, focusJ, slope_gain);
    float gamutBoundaryM = aces_find_gamut_boundary_intersection0(JMcusp, gamma_top_inv, gamma_bottom_inv, J_intersect_source, J_intersect_cusp, gamut_slope);
    if (gamutBoundaryM <= 0.0)
    {
      return vec3(J, 0.0, h);
    }
    float reachBoundaryM = 382.487122 * pow(J_intersect_source / 382.487122,  0.879464149);
    reachBoundaryM = reachBoundaryM / ((382.487122 / reachMaxM) - gamut_slope);
    float remapped_M = aces_remap_M_fwd0(M, gamutBoundaryM, reachBoundaryM);
    float remapped_J = J_intersect_source + remapped_M * gamut_slope;
    return vec3(remapped_J, remapped_M, h);
  }
}

// Declaration of the OCIO shader function

vec4 antitone_render(vec4 inPixel)
{
  vec4 outColor = inPixel;
  
  // Add Range processing
  
  {
    outColor.rgb = max(vec3(0., 0., 0.), outColor.rgb);
    outColor.rgb = min(vec3(1024., 1024., 1024.), outColor.rgb);
  }
  
  // Add Matrix processing
  
  {
    vec4 res = vec4(outColor.rgb.r, outColor.rgb.g, outColor.rgb.b, outColor.a);
    vec4 tmp = res;
    res = mat4(0.69545224135745187, 0.044794563372037723, -0.0055258825581135443, 0., 0.14067869647029416, 0.85967111845642175, 0.0040252103059786595, 0., 0.16386906217225405, 0.0955343181715404, 1.0015006722521349, 0., 0., 0., 0., 1.) * tmp;
    outColor.rgb = vec3(res.x, res.y, res.z);
    outColor.a = res.w;
  }
  
  // Add FixedFunction 'ACES_OutputTransform20 (Forward)' processing
  
  {
    
    // Add RGB to JMh
    
    vec3 JMh;
    vec3 Aab;
    {
      {
        vec3 lms = mat3(0.445181042, 0.123734146, 0.0117007261, 0.34964928, 0.613643706, 0.0280607939, -0.00112973328, 0.0563228093, 0.753939092) * outColor.rgb;
        vec3 F_L_v = pow(abs(lms), vec3(0.419999987, 0.419999987, 0.419999987));
        vec3 rgb_a = (sign(lms) * F_L_v) / ( 27.1299992 + F_L_v);
        Aab = mat3(20.2588081, 15480., 1720., 10.1294041, -16887.2734, 1720., 0.506470203, 1407.27271, -3440.) * rgb_a.rgb;
      }
      {
        if (Aab.r <= 0.0)
        {
          JMh.rgb = vec3(0., 0., 0.);
        }
        else
        {
          float J = 100. * pow(Aab.r, 1.13705599);
          float M = (J == 0.0) ? 0.0 : sqrt(Aab.g * Aab.g + Aab.b * Aab.b);
          float h = (Aab.g == 0.0) ? 0.0 : atan(Aab.b, Aab.g) * 57.29577951308238;
          h = h - floor(h / 360.0) * 360.0;
          h = (h < 0.0) ? h + 360.0 : h;
          JMh.rgb = vec3(J, M, h);
        }
      }
      outColor.rgb = JMh;
    }
    float h_rad = outColor.b * 0.0174532924;
    float cos_hr = cos(h_rad);
    float sin_hr = sin(h_rad);
    
    // Add ToneScale and ChromaCompress (fwd)
    
    float J_ts = aces_tonescale_fwd0(outColor.r);
    // Sample tables (fwd)
    float reachMaxM = aces_reach_m_table_0_sample(outColor.b);
    
    {
      float J = outColor.r;
      float M = outColor.g;
      float h = outColor.b;
      float M_cp = M;
      if (M != 0.0)
      {
        float nJ = J_ts / 382.487122;
        float snJ = max(0.0, 1.0 - nJ);
        float Mnorm;
        {
          float cos_hr2 = 2.0 * cos_hr * cos_hr - 1.0;
          float sin_hr2 = 2.0 * cos_hr * sin_hr;
          float cos_hr3 = 4.0 * cos_hr * cos_hr * cos_hr - 3.0 * cos_hr;
          float sin_hr3 = 3.0 * sin_hr - 4.0 * sin_hr * sin_hr * sin_hr;
          vec3 cosines = vec3(cos_hr, cos_hr2, cos_hr3);
          vec3 cosine_weights = vec3(36.042924962596892, 52.341524240062242, 25.056187951040268);
          vec3 sines = vec3(sin_hr, sin_hr2, sin_hr3);
          vec3 sine_weights = vec3(46.606232166101933, -20.252168130741119, 29.219103959283828);
          Mnorm = dot(cosines, cosine_weights) + dot(sines, sine_weights) + 245.13023138946537;
        }
        float limit = pow(nJ, 0.879464149) * reachMaxM / Mnorm;
        M_cp = M * pow(J_ts / J, 0.879464149);
        M_cp = M_cp / Mnorm;
        M_cp = limit - aces_toe_fwd0(limit - M_cp, limit - 0.001, snJ * 0.200000003, sqrt(nJ * nJ + 0.000250000012));
        M_cp = aces_toe_fwd0(M_cp, limit, nJ * 12.7041578, snJ);
        M_cp = M_cp * Mnorm;
      }
      outColor.rgb = vec3(J_ts, M_cp, h);
    }
    
    // Add GamutCompress (fwd)
    
    {
      vec3 JMGcusp = aces_gamut_cusp_table_0_sample(outColor.b);
      outColor.rgb = aces_gamut_compress0(outColor.rgb, outColor.r, JMGcusp, reachMaxM);
    }
    
    // Add JMh to RGB
    
    {
      vec3 JMh = outColor.rgb;
      vec3 Aab;
      {
        Aab.r = pow(JMh.r * 0.00999999978, 0.879464149);
        Aab.g = JMh.g * cos_hr;
        Aab.b = JMh.g * sin_hr;
      }
      {
        vec3 rgb_a = mat3(0.0323680304, 0.0323680304, 0.0323680304, 2.07657631e-05, -4.10250432e-05, -1.01296409e-05, 1.3260621e-05, -1.20174373e-05, -0.000290076074) * Aab.rgb;
        vec3 rgb_a_lim = min( abs(rgb_a), vec3(0.99000001, 0.99000001, 0.99000001) );
        vec3 lms = sign(rgb_a) * pow( 27.1299992 * rgb_a_lim / (1.0f - rgb_a_lim), vec3(2.38095236, 2.38095236, 2.38095236));
        JMh.rgb = mat3(4.18920946, -0.841454685, 0.00181781233, -2.83307672, 2.44025636, -0.111337893, -0.0962110385, -0.338880897, 1.36944115) * lms;
      }
      outColor.rgb = JMh;
    }
  }
  
  // Add Range processing
  
  {
    outColor.rgb = max(vec3(0., 0., 0.), outColor.rgb);
    outColor.rgb = min(vec3(20., 20., 20.), outColor.rgb);
  }

  return outColor;
}
